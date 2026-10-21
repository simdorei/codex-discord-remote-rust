# Approval form handling

`DISCORD_APPROVAL_POLICY=allow` accepts command, file, permission and MCP URL/empty-form approvals. Empty approval forms may include string `title`, `description`, `$schema`, `$id` and `$comment` fields. These fields must not turn an approval into an automatic cancellation.

The default `ask` policy delivers external MCP empty-form approvals to the original owner, including unknown connectors or risk metadata. A generated button answers the exact request occurrence. Existing saved app grants and low-risk local-tool access retain their behavior. Forms requiring input values or unsupported validation constraints are separate from approvals; the bridge cannot synthesize their answers.

Run the approval regression gate with:

```powershell
cargo test --locked -p cdr-runtime --lib server_request_worker
cargo test --locked -p cdr-runtime --test server_prompt_contract --test component_reply_contract --test bound_component_selection_contract
```

The gate covers annotation parsing, actual resident responses, prompt delivery and receipt reuse, and original owner/generation/occurrence checks. The TypeScript companion repository provides the equivalent `approval-forms` test profile.
