from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.settings import load_gateway_settings

app = create_app(load_gateway_settings())
