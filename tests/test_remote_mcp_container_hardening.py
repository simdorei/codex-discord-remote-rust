from __future__ import annotations

from pathlib import Path


DOCKERFILE = Path("remote_mcp_server/Dockerfile")
COMPOSE_FILE = Path("remote_mcp_server/compose.yaml")
PYTHON_IMAGE = (
    "python:3.12.13-slim@"
    "sha256:57cd7c3a7a273101a6485ba99423ee568157882804b1124b4dd04266317710de"
)
UV_IMAGE = (
    "ghcr.io/astral-sh/uv:0.11.26@"
    "sha256:3d868e555f8f1dbc324afa005066cd11e1053fc4743b9808ca8025283e65efa5"
)


def test_gateway_build_images_are_pinned_by_version_and_digest() -> None:
    dockerfile = DOCKERFILE.read_text(encoding="utf-8")

    assert f"FROM {PYTHON_IMAGE}\n" in dockerfile
    assert f"COPY --from={UV_IMAGE} /uv /uvx /bin/" in dockerfile


def test_gateway_image_declares_fixed_non_root_user() -> None:
    dockerfile = DOCKERFILE.read_text(encoding="utf-8")

    assert "groupadd --gid 10001 simdorei" in dockerfile
    assert "useradd --uid 10001" in dockerfile
    assert "ENV PYTHONDONTWRITEBYTECODE=1" in dockerfile
    assert "USER 10001:10001" in dockerfile


def test_compose_migrates_existing_oauth_volume_before_gateway_start() -> None:
    compose = COMPOSE_FILE.read_text(encoding="utf-8")

    assert "oauth-data-init:" in compose
    assert 'user: "0:0"' in compose
    assert "chown -R --no-dereference 10001:10001 /data" in compose
    assert "condition: service_completed_successfully" in compose


def test_gateway_container_has_bounded_privileges_and_write_paths() -> None:
    compose = COMPOSE_FILE.read_text(encoding="utf-8")

    assert 'user: "10001:10001"' in compose
    assert "cap_drop:\n      - ALL" in compose
    assert "no-new-privileges:true" in compose
    assert "read_only: true" in compose
    assert "/tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777" in compose
    assert "pids_limit: 256" in compose
    assert "init: true" in compose
