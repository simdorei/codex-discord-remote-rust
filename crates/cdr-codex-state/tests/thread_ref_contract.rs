use cdr_codex_state::{ThreadInfo, ThreadResolveError, resolve_thread_ref};

fn thread(id: &str, cwd: &str) -> ThreadInfo {
    ThreadInfo {
        id: id.into(),
        cwd: cwd.into(),
        ..ThreadInfo::default()
    }
}

#[test]
fn prefix_and_workspace_collision_is_ambiguous_not_a_different_target() {
    let threads = vec![
        thread("abcd0000-1111-2222-3333-444444444444", "C:/first"),
        thread("other-id", "C:/abcd"),
    ];
    assert!(matches!(
        resolve_thread_ref(&threads, "abcd", None, false),
        Err(ThreadResolveError::Ambiguous { .. })
    ));
}

#[test]
fn every_displayed_alias_round_trips_even_when_names_are_reserved() {
    for name in [
        "abcd",
        "1",
        "other",
        "next",
        "abcd0000-1111-2222-3333-444444444444",
        "id-a",
        " 1",
        " other",
        " next",
        "\u{2003}1",
        "my project",
        "x|y",
    ] {
        let threads = vec![
            thread("abcd0000-1111-2222-3333-444444444444", "C:/first"),
            thread("id-a", "C:/alpha"),
            thread("id-b", &format!("C:/{name}")),
        ];
        let aliases = cdr_codex_state::workspace_ref_map(&threads);
        for target in &threads {
            assert_eq!(
                resolve_thread_ref(&threads, &aliases[&target.id], Some("id-b"), false)
                    .unwrap()
                    .id,
                target.id,
                "displayed alias for {name} must identify its own row"
            );
        }
    }
}

#[test]
fn numeric_workspace_path_and_full_id_references_match_python() {
    let threads = vec![
        thread("id-a", r"C:\repos\same"),
        thread("id-b", r"C:\repos\project"),
    ];

    assert_eq!(
        resolve_thread_ref(&threads, "1", None, false).unwrap().id,
        "id-a"
    );
    assert_eq!(
        resolve_thread_ref(&threads, "project", None, false)
            .unwrap()
            .id,
        "id-b"
    );
    assert_eq!(
        resolve_thread_ref(&threads, r"c:/REPOS/project", None, false)
            .unwrap()
            .id,
        "id-b"
    );
    assert_eq!(
        resolve_thread_ref(&threads, "id-a", None, false)
            .unwrap()
            .id,
        "id-a"
    );
}

#[test]
fn duplicate_workspaces_require_numbered_refs_and_other_skips_selected() {
    let threads = vec![
        thread("id-a", r"C:\one\same"),
        thread("id-b", r"D:\two\same"),
        thread("id-c", r"C:\other"),
    ];

    assert_eq!(
        resolve_thread_ref(&threads, "same:1", None, false)
            .unwrap()
            .id,
        "id-a"
    );
    assert_eq!(
        resolve_thread_ref(&threads, "same:2", None, false)
            .unwrap()
            .id,
        "id-b"
    );
    assert!(matches!(
        resolve_thread_ref(&threads, "same", None, false),
        Err(ThreadResolveError::Ambiguous { .. })
    ));
    assert_eq!(
        resolve_thread_ref(&threads, "other", Some("id-a"), false)
            .unwrap()
            .id,
        "id-b"
    );
    assert_eq!(
        resolve_thread_ref(&threads, "next", Some("id-b"), false)
            .unwrap()
            .id,
        "id-a"
    );
}

#[test]
fn empty_out_of_range_not_found_and_no_alternate_are_distinct() {
    assert_eq!(
        resolve_thread_ref(&[], "1", None, false).unwrap_err(),
        ThreadResolveError::NoThreads { archived: false }
    );
    let threads = vec![thread("only", "C:/one")];
    assert!(matches!(
        resolve_thread_ref(&threads, "9", None, true),
        Err(ThreadResolveError::IndexOutOfRange { archived: true, .. })
    ));
    assert_eq!(
        resolve_thread_ref(&threads, "next", Some("only"), false).unwrap_err(),
        ThreadResolveError::NoAlternate
    );
    assert!(matches!(
        resolve_thread_ref(&threads, "missing", None, false),
        Err(ThreadResolveError::NotFound(_))
    ));
}

#[test]
fn exact_id_wins_over_another_threads_workspace_alias() {
    let threads = vec![
        thread("other-id", "C:/thread-target"),
        thread("thread-target", "C:/target"),
    ];
    assert_eq!(
        resolve_thread_ref(&threads, "thread-target", None, false)
            .unwrap()
            .id,
        "thread-target"
    );
}

#[test]
fn a_shared_full_workspace_path_is_ambiguous_not_the_first_thread() {
    let threads = vec![thread("id-a", "C:/repo"), thread("id-b", r"c:\repo\")];
    assert!(matches!(
        resolve_thread_ref(&threads, "C:/repo", None, false),
        Err(ThreadResolveError::Ambiguous { .. })
    ));
}

#[test]
fn short_hex_id_requires_a_unique_match() {
    let threads = vec![
        thread("01a079b3-1111-4000-8000-000000000001", "C:/a"),
        thread("01a079b3-2222-4000-8000-000000000002", "C:/b"),
    ];
    assert!(matches!(
        resolve_thread_ref(&threads, "01a079b3", None, false),
        Err(ThreadResolveError::Ambiguous { .. })
    ));
    assert_eq!(
        resolve_thread_ref(&threads, "01a079b3-1111", None, false)
            .unwrap()
            .id,
        threads[0].id
    );
}

#[test]
fn absent_full_uuid_never_resolves_to_a_copy_or_workspace() {
    let id = "01a079b3-1111-4000-8000-000000000001";
    let threads = vec![
        thread(&format!("{id}-bot"), "C:/copy"),
        thread("other", &format!("C:/{id}")),
    ];
    assert!(matches!(
        resolve_thread_ref(&threads, id, None, false),
        Err(ThreadResolveError::NotFound(_))
    ));
}

#[test]
fn case_only_workspace_collisions_get_distinct_usable_aliases() {
    let threads = vec![thread("id-a", "C:/Repo"), thread("id-b", "D:/repo")];
    let refs = cdr_codex_state::workspace_ref_map(&threads);
    assert_eq!(refs["id-a"], "Repo:1");
    assert_eq!(refs["id-b"], "repo:2");
    for item in &threads {
        assert_eq!(
            resolve_thread_ref(&threads, &refs[&item.id], None, false)
                .unwrap()
                .id,
            item.id
        );
    }
}
