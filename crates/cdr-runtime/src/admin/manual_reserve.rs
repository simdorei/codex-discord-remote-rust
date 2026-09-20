use super::args::Args;
use std::path::Path;

pub(super) fn run(args: &Args, root: &Path) -> Result<String, String> {
    let db = super::store::configured_path(root)?;
    if args.command == "retire-automatic-reserve" {
        // Cutover holds its disable/control seal; this additionally proves that
        // no runtime dispatcher can be active for the same installation.
        let _guard = crate::runtime_instance::RuntimeInstanceGuard::acquire(root)
            .map_err(|e| e.to_string())?;
        let result = cdr_store::reserve_retirement::retire(&db, &[]).map_err(|e| e.to_string())?;
        return serde_json::to_string(&result).map_err(|e| e.to_string());
    }
    let request_path = root.join(args.required("--request-file")?);
    let request: cdr_store::final_recovery::Request =
        serde_json::from_slice(&std::fs::read(request_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    cdr_store::final_recovery::authorize(&db, &request, |text| {
        cdr_discord::text::split_delivery_chunks(text, true)
    })
    .map_err(|e| e.to_string())?;
    Ok(format!(
        "saved_final_authorized delivery_id={} job_id={} (no turn submitted)",
        request.delivery_id, request.job_id
    ))
}
