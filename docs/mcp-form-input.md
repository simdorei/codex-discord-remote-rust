# MCP form input

Value-bearing MCP `form` and `openai/form` requests are delivered to the original Discord user and channel. Legacy requests with a requested schema and no mode use form handling too. Neither approval policy automatically supplies values or cancels these requests.

The prompt identifies the requesting server, its message, field names, required fields, types, allowed values and validation constraints. A form with one field accepts a plain value; strings remain strings, and numeric/boolean/array fields accept their JSON value. Forms with several fields take a JSON object:

```json
{"folder":"C:/work","copies":2,"overwrite":false}
```

Use the copyable `[codex-reply:…]` prefix to address the exact request when more than one is pending. Optional fields may be omitted. Defaults are not filled automatically. JSON code blocks are accepted.

Wrong types, missing required fields, invalid selections, bounds, lengths, patterns and formats produce a correction message and leave the original request pending. A successful reply returns `{"action":"accept","content":<validated object>,"_meta":null}`. Plain text `cancel` is a string value, not a cancellation decision. The owner can explicitly use `/cancel` or `/decline`; a literal field value with that spelling can be sent inside a JSON object.

Replies keep the existing owner/channel/active-turn/generation/occurrence checks. A stale, ambiguous or unauthorized reply sends nothing. Repeated delivery or worker recovery reuses the durable prompt receipt. A successfully answered occurrence cannot be answered again.

Validation uses JSON Schema (draft 7 by default, with declared 2019-09 and 2020-12 support), including standard MCP primitive and enum-array fields. Validation performs no coercion, default insertion, field removal, file access or remote schema retrieval. Forms must contain their field schemas directly: reference and asynchronous schema extensions are not supported. Invalid schemas are displayed as pending requests with instructions to complete the interaction in the Codex app or have the plugin correct the schema.

Schema and response content are limited to 64 KiB and 32 nesting levels. Numeric values must be within ±9,007,199,254,740,991 to keep RS/TS validation exact; larger identifiers should use a string schema. A form outside these limits stays pending with feedback, rather than being automatically cancelled.

The same literal fixtures are stored in the RS and TS repositories. They assert protocol payloads, including typed values and validation failures; resident/worker tests additionally verify delivery, retry after invalid input, original authority, stale addressing and exactly one response.

Reference: [MCP elicitation specification](https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation).
