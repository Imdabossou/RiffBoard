# RiffBoard container image.
#
# Small on purpose: RiffBoard only serves feedpaks (no audio conversion), so
# unlike upstream feedBack there is no ffmpeg, vgmstream, fluidsynth or Node
# runtime in the image. The stylesheet is prebuilt and committed.
FROM python:3.12-slim

# Apply the latest Debian security updates to the base layer.
RUN apt-get update \
    && apt-get -y upgrade \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*

WORKDIR /app

COPY requirements.txt .
# pip (and the copies ensurepip keeps) are only needed to build the image;
# they vendor their own urllib3/msgpack/setuptools, which scanners flag. The
# app never installs anything at runtime, so remove them from the final image.
RUN pip install --no-cache-dir -r requirements.txt \
    && pip uninstall -y pip setuptools wheel 2>/dev/null; \
    rm -rf /usr/local/lib/python3.12/ensurepip /usr/local/lib/python3.12/site-packages/pip* \
           /usr/local/lib/python3.12/site-packages/setuptools* /usr/local/bin/pip*

COPY lib/ /app/lib/
COPY static/ /app/static/
COPY plugins/ /app/plugins/
COPY tools/riffboard_admin.py /app/tools/riffboard_admin.py
COPY server.py main.py VERSION /app/

# Unprivileged runtime user. /config (scores, caches, admin token) is the only
# place the app writes; the library at /dlc is only ever read.
RUN useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin riffboard \
    && mkdir -p /config /dlc \
    && chown riffboard:riffboard /config

ENV PYTHONPATH=/app/lib:/app \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    DLC_DIR=/dlc \
    CONFIG_DIR=/config \
    HOST=0.0.0.0 \
    PORT=8000

USER riffboard
EXPOSE 8000
VOLUME ["/config"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD python -c "import urllib.request,sys; sys.exit(0 if urllib.request.urlopen('http://127.0.0.1:8000/api/version', timeout=4).status == 200 else 1)"

CMD ["python", "main.py"]
