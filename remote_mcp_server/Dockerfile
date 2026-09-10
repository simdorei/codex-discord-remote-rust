FROM python:3.12.13-slim@sha256:57cd7c3a7a273101a6485ba99423ee568157882804b1124b4dd04266317710de

COPY --from=ghcr.io/astral-sh/uv:0.11.26@sha256:3d868e555f8f1dbc324afa005066cd11e1053fc4743b9808ca8025283e65efa5 /uv /uvx /bin/

WORKDIR /app

RUN groupadd --gid 10001 simdorei \
    && useradd --uid 10001 --gid 10001 --no-create-home \
        --home-dir /nonexistent --shell /usr/sbin/nologin simdorei \
    && install -d --owner=10001 --group=10001 --mode=0700 /data

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

COPY remote_mcp_server/pyproject.toml remote_mcp_server/uv.lock ./
RUN uv sync --frozen --no-dev

COPY remote_mcp_server /app/remote_mcp_server
COPY simdorei_mcp_common /app/simdorei_mcp_common

USER 10001:10001

EXPOSE 8030

CMD ["/app/.venv/bin/python", "-m", "uvicorn", "remote_mcp_server.simdorei_mcp.main:app", "--host", "0.0.0.0", "--port", "8030", "--proxy-headers", "--forwarded-allow-ips", "127.0.0.1"]
