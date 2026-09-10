from __future__ import annotations

from pathlib import Path

NGINX_CONFIG = Path("remote_mcp_server/nginx-simdorei-mcp.conf")


def test_nginx_bounds_mcp_connections_per_ip_and_globally() -> None:
    config = NGINX_CONFIG.read_text(encoding="utf-8")
    location = _location_block(config, "location = /mcp")

    assert "limit_conn_zone $binary_remote_addr zone=mcp_per_ip:10m;" in config
    assert 'limit_conn_zone "mcp_total_key" zone=mcp_total:10m;' in config
    assert "limit_conn mcp_per_ip 20;" in location
    assert "limit_conn mcp_total 200;" in location
    assert "limit_conn_status 429;" in location


def test_nginx_bounds_bridge_connections_without_breaking_replacement() -> None:
    config = NGINX_CONFIG.read_text(encoding="utf-8")
    location = _location_block(config, "location = /bridge")

    assert "limit_conn_zone $binary_remote_addr zone=bridge_per_ip:10m;" in config
    assert 'limit_conn_zone "bridge_total_key" zone=bridge_total:10m;' in config
    assert "limit_conn bridge_per_ip 32;" in location
    assert "limit_conn bridge_total 64;" in location
    assert "limit_conn_status 429;" in location


def test_nginx_bounds_slow_request_header_and_body_reads() -> None:
    config = NGINX_CONFIG.read_text(encoding="utf-8")

    assert "client_header_timeout 15s;" in config
    assert "client_body_timeout 30s;" in config


def _location_block(config: str, declaration: str) -> str:
    start = config.index(f"    {declaration} {{")
    end = config.index("\n    }", start)
    return config[start:end]
