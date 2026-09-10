CREATE TABLE threads (
    id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER,
    rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER,
    archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT
);
INSERT INTO threads VALUES
    ('thread-a','First title','C:/repos/alpha',20,'a.jsonl','gpt-5','high',100,0,0,'vscode','user'),
    ('thread-b','Second title','C:/repos/beta',10,'b.jsonl','gpt-5','low',50,0,0,'vscode','user'),
    ('thread-old','Old title','C:/repos/old',5,'old.jsonl','gpt-4','medium',25,1,30,'vscode','user');
