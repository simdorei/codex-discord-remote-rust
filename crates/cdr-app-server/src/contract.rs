pub const USED_CLIENT_REQUESTS: &[&str] = &[
    "account/rateLimits/read",
    "account/usage/read",
    "model/list",
    "thread/archive",
    "thread/backgroundTerminals/clean",
    "thread/goal/get",
    "thread/list",
    "thread/read",
    "thread/resume",
    "thread/settings/update",
    "thread/start",
    "thread/unsubscribe",
    "turn/interrupt",
    "turn/start",
    "turn/steer",
];

pub const USED_SERVER_REQUESTS: &[&str] = &[
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/permissions/requestApproval",
    "item/tool/requestUserInput",
    "mcpServer/elicitation/request",
];

pub const USED_NOTIFICATIONS: &[&str] = &[
    "item/completed",
    "item/reasoning/summaryTextDelta",
    "thread/goal/updated",
    "thread/started",
    "turn/completed",
    "turn/started",
];
