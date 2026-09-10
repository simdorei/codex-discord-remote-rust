use super::*;
use serde_json::json;

#[test]
fn persisted_input_requires_same_thread_and_exact_user_input_not_assistant_text() {
    let temp=tempfile::tempdir().unwrap();
    let rollout=temp.path().join("rollout.jsonl");
    let digest=hex::encode(Sha256::digest("요청".as_bytes()));
    for (role,text,expected) in [("user","요청",true),("assistant","요청",false),("user","다른 요청",false)] {
        std::fs::write(&rollout,format!("{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":"target"}}),
            json!({"type":"response_item","payload":{"type":"message","role":role,"content":[{"type":"input_text","text":text}]}})
        )).unwrap();
        assert_eq!(persisted_input(&rollout,"target",&digest).unwrap(),expected);
        assert!(persisted_input(&rollout,"different-target",&digest).is_err());
    }
}

#[test]
fn event_message_input_is_supported_but_missing_metadata_or_partial_json_is_not_proof() {
    let temp=tempfile::tempdir().unwrap();
    let rollout=temp.path().join("rollout.jsonl");
    let digest=hex::encode(Sha256::digest(b"request"));
    let user=json!({"type":"event_msg","payload":{"type":"user_message","message":"request"}});
    std::fs::write(&rollout,format!("{user}\n")).unwrap();
    assert!(!persisted_input(&rollout,"target",&digest).unwrap());
    let meta=json!({"type":"session_meta","payload":{"id":"target"}});
    std::fs::write(&rollout,format!("{meta}\n{user}\n")).unwrap();
    assert!(persisted_input(&rollout,"target",&digest).unwrap());
    std::fs::write(&rollout,format!("{meta}\n{{\"type\":")).unwrap();
    assert!(!persisted_input(&rollout,"target",&digest).unwrap());
}
