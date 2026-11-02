"""Public-site request guard: an ALLOWLIST of what visitors may reach.

RiffBoard is meant to sit on the open internet. Upstream feedBack was built for
one trusted user on a LAN, and its optional demo mode guarded the server with a
blocklist: every route not named on the list stayed open, so a route added
later (or one somebody forgot) was reachable by default. This guard inverts
that. A request is served only when its method and path match an entry below;
everything else gets a plain 404, before any route code runs.

The guard also covers WebSockets, sets the response security headers, and owns
the "who is the client" question for rate limiting (see client_ip()).

Keep the table small and boring. If a new feature needs a route, add the exact
shape here; never add a wildcard that reaches /api/* as a whole.
"""

from __future__ import annotations

import ipaddress
import os
import re

# Shapes for path segments. Library filenames are URL-encoded relative paths,
# so a "rest" segment may contain anything except a NUL; containment against
# the library root is enforced by the route itself (dlc_paths/safepath).
_REST = r".+"
_ID = r"[A-Za-z0-9_.-]+"

# (methods, compiled path regex). Paths are matched against the RAW path as the
# server received it, after percent-decoding by the ASGI server.
_HTTP_ALLOW: list[tuple[frozenset[str], re.Pattern[str]]] = [
    (frozenset({"GET", "HEAD"}), re.compile(p)) for p in (
        r"/",
        r"/favicon\.ico",
        r"/robots\.txt",
        r"/static/.+",
        r"/api/version",
        r"/api/startup-status",
        r"/api/startup-status/stream",
        r"/api/scan-status",
        r"/api/settings",
        r"/api/tunings",
        r"/api/library",
        r"/api/library/(albums|artists|stats|genres|tuning-names|providers)",
        rf"/api/library/providers/[A-Za-z0-9_.:-]+/songs/{_REST}/art",
        rf"/api/work/{_REST}/charts",
        rf"/api/song/{_REST}",          # song info + /art (same shape)
        rf"/api/sloppak/{_REST}/file/{_REST}",
        rf"/audio/{_REST}",
        r"/api/plugins",
        rf"/api/plugins/{_ID}/(screen\.js|screen\.html|settings\.html)",
        rf"/api/plugins/{_ID}/(assets|src)/{_REST}",
        rf"/api/plugins/tuner/(utils|workers|visualization|viz-assets)/{_ID}",
        rf"/api/scores/{_REST}",
        r"/api/scores",
        r"/api/leaderboards",
    )
] + [
    # The only two writes a visitor can make: start a run, submit its score.
    (frozenset({"POST"}), re.compile(r"/api/runs")),
    (frozenset({"POST"}), re.compile(r"/api/scores")),
]

_WS_ALLOW: list[re.Pattern[str]] = [
    re.compile(rf"/ws/highway/{_REST}"),
]

# Operator-only routes. Reached only from loopback AND with the admin token (see
# admin_token()); tools/riffboard_admin.py is the client. Never public.
_ADMIN_PREFIX = "/api/admin/"

_SECURITY_HEADERS: list[tuple[bytes, bytes]] = [
    (b"x-content-type-options", b"nosniff"),
    (b"referrer-policy", b"strict-origin-when-cross-origin"),
    (b"x-frame-options", b"DENY"),
    # Microphone is the point of the app (note detection); everything else off.
    (b"permissions-policy",
     b"microphone=(self), camera=(), geolocation=(), payment=(), usb=(), midi=(self)"),
    (b"cross-origin-opener-policy", b"same-origin"),
]


def is_allowed(method: str, path: str) -> bool:
    if ".." in path.split("/"):
        return False
    for methods, pattern in _HTTP_ALLOW:
        if method in methods and pattern.fullmatch(path):
            return True
    return False


def is_ws_allowed(path: str) -> bool:
    if ".." in path.split("/"):
        return False
    return any(p.fullmatch(path) for p in _WS_ALLOW)


def _is_loopback(host: str | None) -> bool:
    if not host:
        return False
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return host == "localhost"


def _header(scope, name: bytes) -> str | None:
    for k, v in scope.get("headers") or []:
        if k == name:
            try:
                return v.decode("latin-1").strip()
            except Exception:
                return None
    return None


def client_ip(scope) -> str:
    """The visitor's IP for rate limiting.

    Behind a reverse proxy every request arrives from the proxy, so the real
    address is in a header the proxy sets. Which header (if any) to trust is an
    operator decision: RIFFBOARD_CLIENT_IP_HEADER (e.g. ``CF-Connecting-IP``).
    When unset, the socket peer is used, which is right for a direct deploy and
    merely coarse (one bucket for everyone) behind an unconfigured proxy.
    """
    hdr = os.environ.get("RIFFBOARD_CLIENT_IP_HEADER", "").strip().lower()
    if hdr:
        val = _header(scope, hdr.encode("latin-1"))
        if val:
            # X-Forwarded-For style lists: the left-most is the client.
            cand = val.split(",")[0].strip()
            try:
                return str(ipaddress.ip_address(cand))
            except ValueError:
                pass
    client = scope.get("client")
    return client[0] if client else "unknown"


def is_admin_request(scope) -> bool:
    """Loopback peer, no proxy forwarding headers, and the right token."""
    client = scope.get("client")
    if not client or not _is_loopback(client[0]):
        return False
    # A local reverse proxy would make public traffic look like loopback; any
    # forwarding header means "this came from somewhere else".
    for h in (b"x-forwarded-for", b"cf-connecting-ip", b"forwarded", b"x-real-ip"):
        if _header(scope, h) is not None:
            return False
    import admin_token
    return admin_token.check(_header(scope, b"x-riffboard-admin"))


class PublicGuard:
    """ASGI middleware enforcing the allowlist + security headers."""

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        kind = scope.get("type")
        if kind == "http":
            method = scope.get("method", "GET").upper()
            path = scope.get("path", "")
            admin = path.startswith(_ADMIN_PREFIX)
            ok = is_admin_request(scope) if admin else is_allowed(method, path)
            if not ok:
                await _deny_http(send)
                return

            async def send_with_headers(message):
                if message.get("type") == "http.response.start":
                    headers = list(message.get("headers") or [])
                    present = {k.lower() for k, _ in headers}
                    for k, v in _SECURITY_HEADERS:
                        if k not in present:
                            headers.append((k, v))
                    message = {**message, "headers": headers}
                await send(message)

            await self.app(scope, receive, send_with_headers)
            return
        if kind == "websocket":
            if not is_ws_allowed(scope.get("path", "")):
                await send({"type": "websocket.close", "code": 1008})
                return
        await self.app(scope, receive, send)


async def _deny_http(send):
    body = b'{"error":"not found"}'
    await send({
        "type": "http.response.start",
        "status": 404,
        "headers": [
            (b"content-type", b"application/json"),
            (b"content-length", str(len(body)).encode()),
            *_SECURITY_HEADERS,
        ],
    })
    await send({"type": "http.response.body", "body": body})
