#!/usr/bin/env python3
"""Build a small, deterministic feedpak library that exercises the server.

Used to record the reference ("golden") responses from the original Python
server and to test the Rust server against them. Every feature the player
reads from a pack appears at least once: several arrangements (lead + bass),
chords + templates + hand shapes, anchors, bends/slides/techniques, a
two-level difficulty ladder, beats/sections/tempos/time signatures via
song_timeline.json, lyrics, a full mix + instrument stems, tone changes, a
keys.json track (scale degrees), authors, plus a plain one-part bass pack
like FretWright writes and a directory-form pack.

Usage: make_fixtures.py OUT_DIR
"""

import json
import sys
import zipfile
from pathlib import Path

import yaml

FAKE_OGG = b"OggS" + bytes(60)          # the server never decodes audio
FAKE_JPG = bytes.fromhex("ffd8ffe000104a46494600010100000100010000ffd9")
FAKE_PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d4948445200000001000000010806000000"
    "1f15c4890000000d49444154789c6360000002000154a24f5d0000000049454e44ae426082")


def notes(n, start=1.0, step=0.5, string_mod=4, extra=None):
    out = []
    for i in range(n):
        d = {"t": round(start + i * step, 3), "s": i % string_mod, "f": (i * 3) % 12,
             "sus": 0.25 if i % 5 == 0 else 0.0}
        if i % 7 == 0:
            d.update(bn=1.0, bt=0, bnv=[{"t": 0.0, "v": 0.0}, {"t": 0.1, "v": 1.0}])
        if i % 11 == 0:
            d.update(sl=d["f"] + 2)
        if i % 13 == 0:
            d.update(ho=True)
        if i % 17 == 0:
            d.update(pm=True, fg=1)
        if extra:
            d.update(extra(i))
        out.append(d)
    return out


def lead_arrangement():
    chords = []
    for i in range(6):
        t = round(20.0 + i * 1.0, 3)
        chords.append({"t": t, "id": i % 2, "hd": i % 3 == 0,
                       "notes": [{"s": 0, "f": 3, "sus": 0.5}, {"s": 1, "f": 5}, {"s": 2, "f": 5}]})
    chords[1]["fn"] = {"rn": "IV", "q": "maj", "deg": 5}
    lvl_easy = {"difficulty": 0, "notes": notes(3, start=1.0, step=1.0, string_mod=6),
                "chords": [], "anchors": [{"time": 1.0, "fret": 1, "width": 4}], "handshapes": []}
    lvl_hard = {"difficulty": 1, "notes": notes(6, start=1.0, step=0.5, string_mod=6),
                "chords": chords[:2], "anchors": [{"time": 1.0, "fret": 3, "width": 5}],
                "handshapes": [{"chord_id": 0, "start_time": 20.0, "end_time": 20.9, "arp": False}]}
    return {
        "name": "Lead",
        "tuning": [0, 0, 0, 0, 0, 0],
        "capo": 0,
        "notes": notes(40, start=1.0, step=0.45, string_mod=6),
        "chords": chords,
        "anchors": [{"time": 1.0, "fret": 3, "width": 4}, {"time": 20.0, "fret": 1, "width": 5}],
        "handshapes": [{"chord_id": 0, "start_time": 20.0, "end_time": 20.9, "arp": False},
                       {"chord_id": 1, "start_time": 21.0, "end_time": 21.9, "arp": True}],
        "templates": [
            {"name": "G5", "displayName": "G5", "arp": False, "fingers": [1, 3, 4, -1, -1, -1],
             "frets": [3, 5, 5, -1, -1, -1], "voicing": "power", "caged": "E"},
            {"name": "C", "arp": False, "fingers": [-1, 3, 2, 0, 1, 0],
             "frets": [-1, 3, 2, 0, 1, 0], "guideTones": [4, 11, 99]},
        ],
        "phrases": [
            {"start_time": 0.0, "end_time": 10.0, "max_difficulty": 1, "levels": [lvl_easy, lvl_hard]},
            {"start_time": 10.0, "end_time": 30.0, "max_difficulty": 0, "levels": [lvl_easy]},
        ],
        "tones": {"base": "Clean", "changes": [{"t": 12.5, "name": "Drive"}, {"t": 5.0, "name": "Crunch", "rig": "r1"},
                                               {"t": "bad", "name": "Skip"}]},
    }


def bass_arrangement(n=60):
    return {
        "name": "Bass",
        "tuning": [0, 0, 0, 0],
        "capo": 0,
        "notes": notes(n, start=0.8, step=0.4, string_mod=4,
                       extra=lambda i: {"rh": 1, "pkd": 0} if i % 9 == 0 else {}),
        "chords": [], "anchors": [{"time": 0.0, "fret": 1, "width": 4}],
        "handshapes": [], "templates": [],
        "beats": [{"time": 0.5 * i, "measure": (i // 4 + 1) if i % 4 == 0 else -1} for i in range(80)],
        "sections": [{"name": "intro", "number": 1, "time": 0.0}, {"name": "verse", "number": 1, "time": 16.0}],
    }


def write_zip(path: Path, files: dict):
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for name in sorted(files):
            data = files[name]
            if isinstance(data, (dict, list)):
                data = json.dumps(data, indent=1)
            if isinstance(data, str):
                data = data.encode()
            z.writestr(zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0)), data)


def main(out: Path):
    out.mkdir(parents=True, exist_ok=True)

    # 1. Rich two-part song.
    manifest = {
        "title": "Rich Song", "artist": "Fixture Band", "album": "Test Album", "year": 2025,
        "duration": 40.0, "feedpak_version": "1.17.0", "genres": ["Rock", "Indie"], "track": 2,
        "authors": [{"name": "Charter One", "role": "charter", "email": "x@example.com"},
                    {"name": "  "}, "bad", {"name": "Second"}],
        "arrangements": [
            {"id": "lead", "name": "Lead", "file": "arrangements/lead.json", "type": "guitar"},
            {"id": "bass", "name": "Bass", "file": "arrangements/bass.json", "type": "bass",
             "tones": {"base": "Bass Amp"}},
        ],
        "stems": [
            {"id": "full", "file": "stems/full.ogg"},
            {"id": "bass", "file": "stems/bass.ogg", "name": "Bass DI"},
            {"id": "other", "file": "stems/other.ogg", "default": False},
        ],
        "song_timeline": "song_timeline.json",
        "lyrics": "lyrics.json", "lyrics_source": "authored",
        "keys": "keys.json",
        "cover": "cover.png",
    }
    timeline = {
        "beats": [{"time": round(0.5 * i, 3), "measure": (i // 4 + 1) if i % 4 == 0 else -1} for i in range(84)],
        "sections": [{"name": "intro", "number": 1, "time": 0.0}, {"name": "chorus", "number": 1, "time": 20.0}],
        "tempos": [{"time": 0.0, "bpm": 120}, {"time": 20.0, "bpm": 128.5}],
        "time_signatures": [{"time": 0.0, "ts": "4/4"}],
    }
    lyrics = [{"t": 1.0, "d": 0.4, "w": "Hel-"}, {"t": 1.4, "d": 0.4, "w": "lo+"},
              {"t": 2.0, "d": 0.5, "w": "world"}, {"t": "x", "d": 1, "w": "bad"}]
    keys = {"version": 1, "events": [{"t": 0.0, "key": "Em", "scale": "minor"}, {"t": 15.0, "key": "G"},
                                     {"t": "bad", "key": "A"}]}
    write_zip(out / "Fixture Band" / "Rich Song.feedpak", {
        "manifest.yaml": yaml.safe_dump(manifest, sort_keys=False),
        "arrangements/lead.json": lead_arrangement(),
        "arrangements/bass.json": bass_arrangement(),
        "song_timeline.json": timeline,
        "lyrics.json": lyrics,
        "keys.json": keys,
        "stems/full.ogg": FAKE_OGG, "stems/bass.ogg": FAKE_OGG, "stems/other.ogg": FAKE_OGG,
        "cover.png": FAKE_PNG,
    })

    # 2. FretWright-style single bass part, full mix only.
    m2 = {"title": "Plain Bass", "artist": "Fixture Band", "album": "Test Album", "year": 2024,
          "duration": 30.5,
          "arrangements": [{"id": "bass_fw", "name": "Bass (FretWright)", "file": "arrangements/bass_fw.json",
                            "tuning": [0, 0, 0, 0]}],
          "stems": [{"id": "full", "file": "stems/full.ogg"}],
          "song_timeline": "song_timeline.json"}
    b = bass_arrangement(45)
    b["name"] = "Bass"
    write_zip(out / "Fixture Band" / "Plain Bass.feedpak", {
        "manifest.yaml": yaml.safe_dump(m2, sort_keys=False),
        "arrangements/bass_fw.json": b,
        "song_timeline.json": {"beats": timeline["beats"][:60], "sections": timeline["sections"]},
        "stems/full.ogg": FAKE_OGG, "cover.jpg": FAKE_JPG, "preview.ogg": FAKE_OGG,
    })

    # 3. Directory-form pack, guitar in Drop D with capo, no stems.
    d = out / "Another Artist" / "Dir Song.feedpak"
    (d / "arrangements").mkdir(parents=True, exist_ok=True)
    m3 = {"title": "Dir Song", "artist": "Another Artist", "album": "", "duration": 12.0,
          "arrangements": [{"id": "rhythm", "name": "Rhythm", "file": "arrangements/rhythm.json",
                            "tuning": [-2, 0, 0, 0, 0, 0], "capo": 2}]}
    (d / "manifest.yaml").write_text(yaml.safe_dump(m3, sort_keys=False))
    r = lead_arrangement()
    for k in ("phrases", "tones"):
        r.pop(k)
    r["name"] = "Rhythm"
    r["notes"] = r["notes"][:10]
    (d / "arrangements" / "rhythm.json").write_text(json.dumps(r))
    print(f"fixtures written to {out}")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
