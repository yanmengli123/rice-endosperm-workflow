//! Stable lock order for multi-session workflow guards.
//!
//! Branch merge must acquire existing runtimes (never insert) so a finishing
//! turn that still holds `workflow` after leaving `running_turns` cannot be
//! detached and replaced by a second runtime.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Lexicographic id order for acquiring more than one session lock.
pub fn session_lock_order<I, S>(ids: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut ids: Vec<String> = ids.into_iter().map(|s| s.as_ref().to_string()).collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Look up `ids` without inserting. Missing entries are skipped.
pub fn existing_in_lock_order<T: Clone>(
    map: &HashMap<String, T>,
    ids: &[String],
) -> Vec<(String, T)> {
    session_lock_order(ids.iter())
        .into_iter()
        .filter_map(|id| map.get(&id).cloned().map(|value| (id, value)))
        .collect()
}

/// Lock existing mutexes in [`session_lock_order`]. A holder (persist /
/// compaction teardown) blocks the caller until it releases — merge must not
/// detach until that wait finishes.
pub async fn lock_existing_in_order<T: Send>(
    locks: &HashMap<String, Arc<tokio::sync::Mutex<T>>>,
    ids: &[String],
) -> Vec<tokio::sync::OwnedMutexGuard<T>> {
    let mut guards = Vec::new();
    for (_, lock) in existing_in_lock_order(locks, ids) {
        guards.push(lock.lock_owned().await);
    }
    guards
}

/// True when any target is in a live turn, approval wait, or review.
pub fn session_targets_busy(
    running: &HashSet<String>,
    awaiting: &HashSet<String>,
    reviewing: &HashSet<String>,
    ids: &[String],
) -> bool {
    ids.iter()
        .any(|id| running.contains(id) || awaiting.contains(id) || reviewing.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn lock_order_is_lexicographic_regardless_of_input_order() {
        assert_eq!(
            session_lock_order(["z-main", "a-branch"]),
            vec!["a-branch".to_string(), "z-main".to_string()]
        );
        assert_eq!(
            session_lock_order(["a-branch", "z-main"]),
            session_lock_order(["z-main", "a-branch"])
        );
        assert_eq!(
            session_lock_order(["b", "a", "b"]),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn existing_lookup_never_inserts_a_second_runtime() {
        let mut map = HashMap::new();
        map.insert("main".into(), 1u8);
        let found = existing_in_lock_order(&map, &["main".into(), "branch".into()]);
        assert_eq!(found, vec![("main".into(), 1)]);
        assert_eq!(map.len(), 1);
        assert!(!map.contains_key("branch"));
    }

    #[test]
    fn merge_refuses_while_busy() {
        let ids = vec!["main".into(), "branch".into()];
        let mut running = HashSet::new();
        running.insert("main".into());
        assert!(session_targets_busy(
            &running,
            &HashSet::new(),
            &HashSet::new(),
            &ids
        ));

        let mut awaiting = HashSet::new();
        awaiting.insert("branch".into());
        assert!(session_targets_busy(
            &HashSet::new(),
            &awaiting,
            &HashSet::new(),
            &ids
        ));

        let mut reviewing = HashSet::new();
        reviewing.insert("main".into());
        assert!(session_targets_busy(
            &HashSet::new(),
            &HashSet::new(),
            &reviewing,
            &ids
        ));

        assert!(!session_targets_busy(
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &ids
        ));
    }

    #[tokio::test]
    async fn held_workflow_lock_blocks_merge_detach() {
        let main = "frame-m".to_string();
        let branch = "frame-b".to_string();
        let main_rt = Arc::new(tokio::sync::Mutex::new(()));
        let branch_rt = Arc::new(tokio::sync::Mutex::new(()));
        let mut sessions = HashMap::new();
        sessions.insert(main.clone(), main_rt.clone());
        sessions.insert(branch.clone(), branch_rt.clone());
        let ids = vec![main.clone(), branch.clone()];

        // Persist/compaction still holds workflow after leaving running_turns.
        let persist = main_rt.clone().lock_owned().await;

        let merge_map = sessions.clone();
        let merge = tokio::spawn({
            let ids = ids.clone();
            let main = main.clone();
            async move {
                let found = existing_in_lock_order(&merge_map, &ids);
                assert_eq!(found.len(), 2);
                let _locks = lock_existing_in_order(&merge_map, &ids).await;
                // Production merge keeps the Arc and only drops the cached agent.
                merge_map.get(&main).cloned()
            }
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !merge.is_finished(),
            "merge must wait for the finishing turn's workflow lock"
        );

        let turn_rt = sessions
            .entry(main.clone())
            .or_insert_with(|| {
                panic!("created a second runtime for a frame still finishing persist")
            })
            .clone();
        assert!(
            Arc::ptr_eq(&turn_rt, &main_rt),
            "same frame_id must reuse the existing runtime"
        );

        drop(persist);
        let kept = tokio::time::timeout(Duration::from_secs(1), merge)
            .await
            .expect("merge should finish once the workflow lock is released")
            .unwrap()
            .expect("main runtime stays in the map");
        assert!(Arc::ptr_eq(&kept, &main_rt));
        assert!(
            Arc::ptr_eq(sessions.get(&main).expect("main runtime"), &main_rt),
            "merge must not replace the runtime Arc"
        );
    }

    #[tokio::test]
    async fn opposite_input_order_does_not_deadlock() {
        let locks: HashMap<_, _> = [
            ("z".into(), Arc::new(tokio::sync::Mutex::new(()))),
            ("a".into(), Arc::new(tokio::sync::Mutex::new(()))),
        ]
        .into_iter()
        .collect();

        let left = {
            let locks = locks.clone();
            tokio::spawn(async move {
                for _ in 0..100 {
                    let _g = lock_existing_in_order(&locks, &["z".into(), "a".into()]).await;
                }
            })
        };
        let right = {
            let locks = locks.clone();
            tokio::spawn(async move {
                for _ in 0..100 {
                    let _g = lock_existing_in_order(&locks, &["a".into(), "z".into()]).await;
                }
            })
        };

        tokio::time::timeout(Duration::from_secs(5), async {
            left.await.unwrap();
            right.await.unwrap();
        })
        .await
        .expect("lexicographic lock order must not deadlock");
    }
}
