import importlib.util
from contextlib import closing
from pathlib import Path
import sqlite3
import tempfile
import unittest


class PreviewTests(unittest.TestCase):
    def test_missing_database_is_not_created(self):
        spec=importlib.util.spec_from_file_location("preview",Path("scripts/inspect_new_first_reply.py"))
        module=importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/"missing.sqlite"
            with self.assertRaises(sqlite3.OperationalError):
                module.inspect(path,"job")
            self.assertFalse(path.exists())

    def test_absent_receipt_never_authorizes_legacy_delivery_and_changes_nothing(self):
        spec=importlib.util.spec_from_file_location("preview",Path("scripts/inspect_new_first_reply.py"))
        module=importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/"mirror.sqlite"
            with closing(sqlite3.connect(path)) as db:
                db.executescript('''
                CREATE TABLE discord_ingress_journal(ingress_id,kind,event_id,channel_id,target_thread_id,outcome_json,confirmation_delivered,owner_id);
                INSERT INTO discord_ingress_journal VALUES('message:1','message',1,42,'thread','{"new_verification":{"thread_id":"thread","channel_id":43}}',0,'job');
                CREATE TABLE codex_delivery_outbox(job_id,target_thread_id,turn_id,channel_id,content);
                INSERT INTO codex_delivery_outbox VALUES('job','thread','turn',43,'private final');
                CREATE TABLE mirror_threads(codex_thread_id,discord_thread_id);
                INSERT INTO mirror_threads VALUES('thread',43);
                CREATE TABLE codex_delivery_receipts(receipt_key,message_id,retryable,blocked_reason);
                ''')
            before=path.read_bytes()
            report=module.inspect(path,"job")
            self.assertEqual(report["normal_ack_receipt"],"absent")
            self.assertTrue(report["destination_matches"])
            self.assertFalse(report["delivery_authorized"])
            self.assertFalse(report["replay_authorized"])
            self.assertNotIn("private final",str(report))
            self.assertEqual(path.read_bytes(),before)
