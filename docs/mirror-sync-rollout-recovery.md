# Manual sync of compressed conversation logs

`!mirror sync` can restore a missing plain conversation log from its exact
`.jsonl.zst` counterpart. Restoration runs only when sync is requested; there is
no new background decompression loop.

- An existing plain log takes precedence. Sync does not replace it with an archive.
- A compressed log is decoded into a temporary file beside the destination. Its
  session ID must match the requested conversation, and its final byte must be a
  newline. The metadata line is limited to 1 MiB. This is not a full JSON validation
  pass over every historical record.
- The complete file is published without overwriting a file created meanwhile.
  The compressed original is retained. Decode, access, identity and publication
  failures are returned as errors instead of being counted as successful syncs.
- A saved read position must use the current rollout path, fall within the file,
  and point to a record boundary or EOF. Sync preserves the position, turn context
  and event history. It never resets a position to zero or advances it to EOF.
- An invalid position stops sync with the conversation ID and the reason, before
  Discord room changes or cleanup. A staged restoration for that conversation is
  not published. The message says `explicit recovery required`; investigate and
  choose a recovery policy rather than repeatedly resetting progress.
- If neither plain nor compressed data exists, the existing `unavailable` result
  remains. Restoring a log does not change the ordinary sync scope or cleanup rules.

Decompression happens outside the mirror DB write lock. Publication validates the
latest saved position while holding the lock used by the background reader.
On Windows the source archive is also pinned against writes and replacement.
The shared lock is advisory on Unix; archive writers must honor it. Source identity,
size and modification time are checked again before publication.

A normal saved position can have unread history behind it. Preserving that position
allows the existing reader to deliver that history; this feature does not silently
discard it. When the position is already at EOF, only subsequently appended records
are delivered. A new conversation without a saved position keeps the existing first
poll behavior.

This change does not add compressed-file support to `mirror check`, context reading,
archive commands, or ordinary background discovery. After logs are compressed again,
request sync to restore them. It also does not diagnose how an old saved position
became invalid or automatically repair such a position.
