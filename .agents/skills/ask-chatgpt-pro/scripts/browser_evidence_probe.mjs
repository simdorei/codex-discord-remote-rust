const PROTOCOL = "ask-chatgpt-pro-browser-evidence-v2";

export async function probeChrome(globals) {
  const agent = globals?.agent;
  if (agent?.browsers == null || typeof agent.browsers.get !== "function") {
    return decision("unverified", false, {
      reason: "Browser runtime was not initialized.",
    });
  }

  if (globals.chrome != null) {
    return available(globals.chrome, globals, "existing_binding");
  }

  let initialError = "";
  try {
    const browser = await agent.browsers.get("chrome");
    return available(browser, globals, "initial_selection");
  } catch (error) {
    initialError = publicError(error);
  }

  try {
    await agent.documentation.get("chrome-troubleshooting");
  } catch (error) {
    return decision("unverified", false, {
      reason: "Browser troubleshooting documentation could not be read.",
      failed_stage: "chrome_troubleshooting",
      public_error: publicError(error),
      initial_error: initialError,
    });
  }

  try {
    const browser = await agent.browsers.get("chrome");
    return available(browser, globals, "retry_selection", initialError);
  } catch (error) {
    return decision("unavailable", true, {
      reason: "The second explicit Chrome selection failed after verified recovery.",
      failed_stage: "select_chrome_retry",
      public_error: publicError(error),
      initial_error: initialError,
    });
  }
}

async function available(browser, globals, selectedStage, initialError = "") {
  globals.chrome = browser;
  let tabCount = null;
  let tabStateError = "";
  try {
    const tabs = await browser.tabs.list();
    tabCount = Array.isArray(tabs) ? tabs.length : null;
  } catch (error) {
    tabStateError = publicError(error);
  }
  return decision("available", false, {
    reason:
      tabCount === 0
        ? "Explicit Chrome selection succeeded; an empty tab list requires opening a tab."
        : "Explicit Chrome selection succeeded.",
    selected_stage: selectedStage,
    initial_error: initialError,
    tab_count: tabCount,
    tab_state_error: tabStateError,
  });
}

function decision(status, canReportUnavailable, details) {
  return {
    protocol: PROTOCOL,
    browser_type: "chrome",
    status,
    can_report_unavailable: canReportUnavailable,
    ...details,
  };
}

function publicError(error) {
  const raw =
    error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  return raw
    .replace(
      /(authorization|bearer|cookie|password|otp|token)\s*[:=]\s*\S+/gi,
      "$1=<redacted>",
    )
    .replace(/codex-project-[A-Za-z0-9_-]{20,}/g, "codex-project-<redacted>")
    .replace(/[\r\n]+/g, " ")
    .trim()
    .slice(0, 500);
}
