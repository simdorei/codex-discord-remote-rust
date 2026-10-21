# Approval form handling

`DISCORD_APPROVAL_POLICY=allow` accepts command, file, permission and MCP URL/empty-form approvals. Empty approval forms may include string `title`, `description`, `$schema`, `$id` and `$comment` fields. These fields must not turn an approval into an automatic cancellation.

The default `ask` policy delivers external MCP empty-form approvals to the original owner, including unknown connectors or risk metadata. A generated button answers the exact request occurrence. Existing saved app grants and low-risk local-tool access retain their behavior. Value-bearing forms are delivered as [MCP input prompts](mcp-form-input.md) under both policies and remain pending until the owner replies. Invalid input is reported without cancelling the request. The bridge never synthesizes answers.

Run the approval regression gate with:

```powershell
cargo test --locked -p cdr-app-server --test mcp_form_input_contract
cargo build --locked -p cdr-runtime --bin cdr-offline-soak
cargo test --locked -p cdr-runtime --lib server_request_worker
cargo test --locked -p cdr-runtime --test server_prompt_contract --test component_reply_contract --test bound_component_selection_contract
```

The gate covers annotation parsing, actual resident responses, prompt delivery and receipt reuse, and original owner/generation/occurrence checks. The TypeScript companion repository provides the equivalent `approval-forms` test profile.
