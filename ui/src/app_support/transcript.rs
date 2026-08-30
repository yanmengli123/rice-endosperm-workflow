use super::*;

const MAX_IDLE_TRANSCRIPT_CACHE: usize = 8;

fn trim_idle_transcript_cache(
    cache: &mut HashMap<String, Vec<ChatItem>>,
    running: &HashSet<String>,
    protected: Option<&str>,
) {
    let idle_count = cache.keys().filter(|id| !running.contains(*id)).count();
    let idle = cache
        .keys()
        .filter(|id| !running.contains(*id) && protected != Some(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let remove = idle_count.saturating_sub(MAX_IDLE_TRANSCRIPT_CACHE);
    // ponytail: arbitrary idle eviction is enough because SQLite is the source
    // of truth; add LRU ordering only if reload latency becomes measurable.
    for id in idle.into_iter().take(remove) {
        cache.remove(&id);
    }
}

/// Replace the visible transcript in one signal write, moving the old rows
/// into the inactive-session cache and taking cached target rows by ownership.
pub(crate) fn replace_visible_transcript(
    current_id: Option<String>,
    target_id: Option<&str>,
    fallback: Vec<ChatItem>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    running: RwSignal<HashSet<String>>,
) {
    if target_id.is_some() && current_id.as_deref() == target_id {
        return;
    }
    let next = transcripts
        .try_update(|cache| {
            target_id
                .and_then(|id| cache.remove(id))
                .unwrap_or(fallback)
        })
        .unwrap_or_default();
    let previous = items
        .try_update(|visible| std::mem::replace(visible, next))
        .unwrap_or_default();
    let running = running.get_untracked();
    transcripts.update(|cache| {
        if let Some(current_id) = current_id.as_ref() {
            cache.insert(current_id.clone(), previous);
        }
        trim_idle_transcript_cache(cache, &running, current_id.as_deref());
    });
}

#[cfg(test)]
mod transcript_cache_tests {
    use super::{
        replace_visible_transcript, trim_idle_transcript_cache, MAX_IDLE_TRANSCRIPT_CACHE,
    };
    use crate::dto::ChatItem;
    use leptos::{create_runtime, create_rw_signal, SignalGetUntracked};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn trims_only_idle_transcripts() {
        let running = HashSet::from(["running".to_string()]);
        let mut cache =
            HashMap::from([("running".to_string(), vec![ChatItem::User("live".into())])]);
        for index in 0..MAX_IDLE_TRANSCRIPT_CACHE + 3 {
            cache.insert(format!("idle-{index}"), Vec::new());
        }
        trim_idle_transcript_cache(&mut cache, &running, None);
        assert!(cache.contains_key("running"));
        assert_eq!(cache.len(), MAX_IDLE_TRANSCRIPT_CACHE + 1);
    }

    #[test]
    fn moves_rows_between_visible_and_cached_owners() {
        let runtime = create_runtime();
        let items = create_rw_signal(vec![ChatItem::User("session-a".into())]);
        let transcripts = create_rw_signal(HashMap::from([(
            "b".to_string(),
            vec![ChatItem::User("session-b".into())],
        )]));
        let running = create_rw_signal(HashSet::new());

        replace_visible_transcript(
            Some("a".into()),
            Some("b"),
            Vec::new(),
            items,
            transcripts,
            running,
        );

        assert!(matches!(
            items.get_untracked().as_slice(),
            [ChatItem::User(text)] if text == "session-b"
        ));
        let cache = transcripts.get_untracked();
        assert!(!cache.contains_key("b"));
        assert!(matches!(
            cache.get("a").map(Vec::as_slice),
            Some([ChatItem::User(text)]) if text == "session-a"
        ));
        runtime.dispose();
    }
}

/// Map the reviewer's `[msg:N]` index to the live UI row. Usage, reviewer
/// handoffs, approvals, and review cards are UI-only and must not shift it.
pub(crate) fn review_message_ui_index(items: &[ChatItem], message_index: usize) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| match item {
            ChatItem::User(text) | ChatItem::Assistant { text, .. } | ChatItem::Reasoning(text) => {
                !text.trim().is_empty()
            }
            ChatItem::BranchMerge { text, .. } => !text.trim().is_empty(),
            ChatItem::Tool { name, .. } => name != "attempt_completion",
            ChatItem::AcpTool { .. } | ChatItem::Plan(_) | ChatItem::Question(_) => true,
            ChatItem::QueuedUser { .. }
            | ChatItem::FileChanged(_)
            | ChatItem::ApprovalPending { .. }
            | ChatItem::AcpPermission { .. }
            | ChatItem::Usage { .. }
            | ChatItem::Compaction { .. }
            | ChatItem::ReviewTransition { .. }
            | ChatItem::Review(_) => false,
        })
        .nth(message_index)
        .map(|(ui_index, _)| ui_index)
}

#[cfg(test)]
mod review_jump_tests {
    use super::{
        browser_offline_notice_from_items, browser_setup_live_retrieval,
        composer_text_from_user_message, last_user_composer_text, review_message_ui_index,
    };
    use crate::dto::{ChatItem, ContextUsage, ReviewTransitionPhase};

    fn assistant(text: &str) -> ChatItem {
        ChatItem::Assistant {
            text: text.into(),
            model: None,
            resources: vec![],
        }
    }

    #[test]
    fn review_indices_ignore_ui_only_rows() {
        let items = vec![
            ChatItem::User("earlier question".into()),
            assistant("earlier answer"),
            ChatItem::Usage {
                input: 10,
                output: 2,
                reasoning: 0,
                cached: 0,
                ctx_tokens: 0,
                max_context: 0,
                context_usage: ContextUsage::default(),
            },
            ChatItem::ReviewTransition {
                phase: ReviewTransitionPhase::Reviewing,
                model: None,
            },
            ChatItem::User("current question".into()),
            assistant("problematic answer"),
        ];

        assert_eq!(review_message_ui_index(&items, 3), Some(5));
    }

    #[test]
    fn editing_a_feedback_turn_excludes_hidden_diagnostics() {
        assert_eq!(
            composer_text_from_user_message(
                "The app froze\n\nFeedback context: \"Wisp version: 0.34.0\""
            ),
            "The app froze"
        );
    }

    #[test]
    fn last_user_composer_text_skips_attachment_suffixes() {
        let items = vec![
            ChatItem::User("latest rustc\n\nUploaded files: a.png".into()),
            ChatItem::Assistant {
                text: "ok".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        assert_eq!(last_user_composer_text(&items), "latest rustc");
    }

    #[test]
    fn browser_offline_notice_tracks_disconnect_then_restore() {
        let mut items = vec![
            ChatItem::User("latest rustc".into()),
            ChatItem::Tool {
                name: "web_scan".into(),
                ok: Some(false),
                input: "tabs".into(),
                output: "real-browser bridge unavailable. WISP_BROWSER_DISCONNECTED".into(),
                started_at_ms: None,
                duration_ms: None,
            },
        ];
        let notice = browser_offline_notice_from_items("s1", &items).unwrap();
        assert_eq!(notice.frame_id, "s1");
        assert_eq!(notice.retry_text, "latest rustc");

        items.push(ChatItem::Tool {
            name: "web_scan".into(),
            ok: Some(true),
            input: "tabs".into(),
            output: "{\"tabs\":[]}".into(),
            started_at_ms: None,
            duration_ms: None,
        });
        assert!(browser_offline_notice_from_items("s1", &items).is_none());
    }

    fn setup_item(live: bool) -> ChatItem {
        ChatItem::Tool {
            name: "browser_setup".into(),
            ok: Some(true),
            input: String::new(),
            output: format!(
                "{{\n  \"status\": \"{}\",\n  \"live_retrieval\": {}\n}}",
                if live { "connected" } else { "disconnected" },
                live
            ),
            started_at_ms: None,
            duration_ms: None,
        }
    }

    #[test]
    fn browser_setup_json_is_the_block_and_restore_signal() {
        let blocked = vec![ChatItem::User("latest rustc".into()), setup_item(false)];
        assert!(browser_offline_notice_from_items("s1", &blocked).is_some());

        let restored = vec![
            ChatItem::User("latest rustc".into()),
            setup_item(false),
            setup_item(true),
        ];
        assert!(
            browser_offline_notice_from_items("s1", &restored).is_none(),
            "connected browser_setup must clear an earlier disconnect"
        );
    }

    #[test]
    fn successful_live_tools_override_an_earlier_disconnected_setup() {
        let items = vec![
            ChatItem::User("CLEC12A pubmed".into()),
            setup_item(false),
            scan_item(true),
            ChatItem::Tool {
                name: "web_execute_js".into(),
                ok: Some(true),
                input: "Date()".into(),
                output: "{\"result\":\"ok\"}".into(),
                started_at_ms: None,
                duration_ms: None,
            },
            ChatItem::Assistant {
                text: "PubMed currently lists hits for CLEC12A.".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        assert!(
            browser_offline_notice_from_items("s1", &items).is_none(),
            "a later successful scan must not keep the offline banner (#887)"
        );
    }

    fn scan_item(ok: bool) -> ChatItem {
        ChatItem::Tool {
            name: "web_scan".into(),
            ok: Some(ok),
            input: "tabs".into(),
            output: if ok {
                "{\"tabs\":[{\"title\":\"PubMed\"}]}".into()
            } else {
                "real-browser bridge unavailable. WISP_BROWSER_DISCONNECTED".to_string()
            },
            started_at_ms: None,
            duration_ms: None,
        }
    }

    #[test]
    fn a_reconnecting_extension_after_a_successful_scan_keeps_the_turn_live() {
        let items = vec![
            ChatItem::User("latest rustc".into()),
            setup_item(false),
            scan_item(true),
            scan_item(false),
        ];
        assert!(
            browser_offline_notice_from_items("s1", &items).is_none(),
            "one successful retrieval means the answer has live results (#921)"
        );
    }

    #[test]
    fn the_notice_describes_the_latest_turn_only() {
        let mut items = vec![
            ChatItem::User("latest rustc".into()),
            scan_item(false),
            ChatItem::Assistant {
                text: "Connect the extension first.".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        assert!(browser_offline_notice_from_items("s1", &items).is_some());

        items.push(ChatItem::User("read this page".into()));
        assert!(
            browser_offline_notice_from_items("s1", &items).is_none(),
            "a new turn starts without an offline verdict"
        );
    }

    #[test]
    fn a_truncated_setup_dump_still_reports_live_retrieval() {
        let truncated = format!(
            "{{\n  \"status\": \"disconnected\",\n  \"live_retrieval\": false,\n  \"steps\": [\"{}",
            "x".repeat(64)
        );
        assert_eq!(browser_setup_live_retrieval(&truncated), Some(false));
        assert_eq!(
            browser_setup_live_retrieval("{\"status\":\"connected\"}"),
            Some(true)
        );
        assert_eq!(browser_setup_live_retrieval("not json"), None);
    }
}

pub(crate) const BROWSER_DISCONNECTED_MARKER: &str = "WISP_BROWSER_DISCONNECTED";

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BrowserOfflineNotice {
    pub frame_id: String,
    pub retry_text: String,
}

/// Apply a recomputed verdict for one session without disturbing another
/// session's banner.
pub(crate) fn set_browser_offline_notice(
    notice: RwSignal<Option<BrowserOfflineNotice>>,
    frame_id: &str,
    next: Option<BrowserOfflineNotice>,
) {
    match next {
        Some(next) => notice.set(Some(next)),
        None => notice.update(|current| {
            if current.as_ref().is_some_and(|row| row.frame_id == frame_id) {
                *current = None;
            }
        }),
    }
}

pub(crate) fn last_user_composer_text(items: &[ChatItem]) -> String {
    items
        .iter()
        .rev()
        .find_map(|item| match item {
            ChatItem::User(text) => Some(composer_text_from_user_message(text)),
            _ => None,
        })
        .unwrap_or_default()
}

fn is_live_retrieval_tool(name: &str) -> bool {
    matches!(
        name,
        "web_scan" | "web_open_tab" | "web_execute_js" | "web_screenshot"
    )
}

pub(crate) fn is_browser_retrieval_tool(name: &str) -> bool {
    name == "browser_setup" || is_live_retrieval_tool(name)
}

/// `browser_setup` returns a long JSON dump that the transcript bounds, so read
/// the flag as text: a truncated copy still carries the head fields.
pub(crate) fn browser_setup_live_retrieval(content: &str) -> Option<bool> {
    if let Some((_, tail)) = content.split_once("\"live_retrieval\"") {
        let value = tail.trim_start().trim_start_matches(':').trim_start();
        if value.starts_with("true") {
            return Some(true);
        }
        if value.starts_with("false") {
            return Some(false);
        }
    }
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    match value.get("status").and_then(|v| v.as_str()) {
        Some("connected") => Some(true),
        Some(_) => Some(false),
        None => None,
    }
}

/// A refused live-retrieval attempt: `browser_setup` reports it as data, the
/// retrieval tools fail with the bridge's disconnect marker.
fn browser_retrieval_blocked(name: &str, ok: bool, content: &str) -> bool {
    if name == "browser_setup" {
        return browser_setup_live_retrieval(content) == Some(false);
    }
    is_live_retrieval_tool(name) && !ok && content.contains(BROWSER_DISCONNECTED_MARKER)
}

/// Live page data actually reached this turn. The disconnect marker is checked
/// because a replayed transcript reports every persisted tool row as ok.
fn browser_retrieval_succeeded(name: &str, ok: bool, content: &str) -> bool {
    is_live_retrieval_tool(name) && ok && !content.contains(BROWSER_DISCONNECTED_MARKER)
}

/// The banner speaks about the answer on screen, so only the latest turn counts.
/// The extension's service worker sleeps and reconnects on a one-minute alarm, so
/// a turn routinely mixes refused attempts with successful ones; one success
/// means the answer does contain live results (#887, #921).
pub(crate) fn browser_offline_notice_from_items(
    frame_id: &str,
    items: &[ChatItem],
) -> Option<BrowserOfflineNotice> {
    let turn_start = items
        .iter()
        .rposition(|item| matches!(item, ChatItem::User(_)))
        .map_or(0, |index| index + 1);
    let mut blocked = false;
    for item in &items[turn_start..] {
        let ChatItem::Tool {
            name,
            ok: Some(ok),
            output,
            ..
        } = item
        else {
            continue;
        };
        if browser_retrieval_succeeded(name, *ok, output) {
            return None;
        }
        if browser_retrieval_blocked(name, *ok, output) {
            blocked = true;
        } else if name == "browser_setup" && browser_setup_live_retrieval(output) == Some(true) {
            blocked = false;
        }
    }
    blocked.then(|| BrowserOfflineNotice {
        frame_id: frame_id.to_string(),
        retry_text: last_user_composer_text(items),
    })
}

pub(crate) fn composer_text_from_user_message(text: &str) -> String {
    [
        "\n\nUploaded files: ",
        "\n\nAttached artifacts: ",
        "\n\nAttached sessions: ",
        "\n\nProject context: ",
        "\n\nSelected skills: ",
        "\n\nSelected workflows: ",
        "\n\nTarget environments: ",
        "\n\nTarget runtimes: ",
        "\n\nAI source-edit instruction: ",
        "\n\nFeedback context: ",
    ]
    .iter()
    .filter_map(|marker| text.find(marker))
    .min()
    .map(|idx| text[..idx].trim().to_string())
    .unwrap_or_else(|| text.to_string())
}

pub(crate) fn user_message_index(items: &[ChatItem], ui_index: usize) -> Option<usize> {
    if !matches!(items.get(ui_index), Some(ChatItem::User(_))) {
        return None;
    }
    Some(
        items
            .iter()
            .take(ui_index + 1)
            .filter(|item| matches!(item, ChatItem::User(_)))
            .count()
            .saturating_sub(1),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueuedTurnRow {
    pub id: u64,
    pub text: String,
    pub user_index: usize,
}

/// Parked follow-ups for the composer card, with outline `data-user-index`
/// values that stay aligned with `user_turn_index`.
pub(crate) fn queued_turn_rows(items: &[ChatItem], user_offset: usize) -> Vec<QueuedTurnRow> {
    let mut n = 0;
    let mut rows = Vec::new();
    for item in items {
        match item {
            ChatItem::User(_) => n += 1,
            ChatItem::QueuedUser { id, text } => {
                rows.push(QueuedTurnRow {
                    id: *id,
                    text: text.clone(),
                    user_index: user_offset + n,
                });
                n += 1;
            }
            _ => {}
        }
    }
    rows
}

pub(crate) fn user_turn_index(items: &[ChatItem], ui_index: usize) -> Option<usize> {
    if !matches!(
        items.get(ui_index),
        Some(ChatItem::User(_) | ChatItem::QueuedUser { .. })
    ) {
        return None;
    }
    Some(
        items
            .iter()
            .take(ui_index + 1)
            .filter(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
            .count()
            .saturating_sub(1),
    )
}

/// Return the stable user-turn index owning any row in that turn. Unlike
/// `user_turn_index`, this intentionally maps assistant/tool rows back to the
/// most recent user row so turn-boundary actions can live on the reply.
pub(crate) fn owning_user_turn_index(items: &[ChatItem], ui_index: usize) -> Option<usize> {
    items
        .iter()
        .take(ui_index + 1)
        .filter(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
        .count()
        .checked_sub(1)
}

pub(crate) fn transcript_item_timestamp(
    items: &[ChatItem],
    ui_index: usize,
    user_offset: usize,
    outline: &[SessionOutlineItem],
) -> Option<i64> {
    let item = items.get(ui_index)?;
    if !matches!(
        item,
        ChatItem::User(_) | ChatItem::QueuedUser { .. } | ChatItem::Assistant { .. }
    ) {
        return None;
    }
    let user_index = items
        .iter()
        .take(ui_index + 1)
        .filter(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
        .count()
        .checked_sub(1)?
        + user_offset;
    let entry = outline
        .iter()
        .find(|entry| entry.user_index == user_index)?;
    match item {
        ChatItem::User(_) | ChatItem::QueuedUser { .. } => {
            entry.sent_at.filter(|timestamp| *timestamp > 0)
        }
        ChatItem::Assistant { .. } => entry.response_at.filter(|timestamp| *timestamp > 0),
        _ => None,
    }
}

pub(crate) fn turn_duration_ms(sent_at: Option<i64>, response_at: Option<i64>) -> Option<u64> {
    let sent_at = sent_at.filter(|timestamp| *timestamp > 0)?;
    let response_at = response_at.filter(|timestamp| *timestamp >= sent_at)?;
    Some(response_at.saturating_sub(sent_at) as u64 * 1_000)
}

pub(crate) fn merge_conversation_outline(
    persisted: &[SessionOutlineItem],
    items: &[ChatItem],
    user_offset: usize,
) -> Vec<SessionOutlineItem> {
    let mut persisted = persisted.to_vec();
    if !persisted
        .windows(2)
        .all(|window| window[0].user_index <= window[1].user_index)
    {
        persisted.sort_by_key(|entry| entry.user_index);
    }
    let live = items
        .iter()
        .filter_map(|item| match item {
            ChatItem::User(text) | ChatItem::QueuedUser { text, .. } => Some(text),
            _ => None,
        })
        .enumerate()
        .map(|(local_index, text)| SessionOutlineItem {
            user_index: user_offset + local_index,
            seq: None,
            text: text.clone(),
            sent_at: None,
            response_at: None,
        })
        .collect::<Vec<_>>();

    // Both inputs are ordered by user index, so merge once instead of finding
    // every live turn in the growing persisted vector and sorting afterwards.
    let mut outline = Vec::with_capacity(persisted.len() + live.len());
    let mut persisted = persisted.into_iter().peekable();
    let mut live = live.into_iter().peekable();
    while let (Some(saved), Some(current)) = (persisted.peek(), live.peek()) {
        match saved.user_index.cmp(&current.user_index) {
            std::cmp::Ordering::Less => outline.push(persisted.next().unwrap()),
            std::cmp::Ordering::Greater => outline.push(live.next().unwrap()),
            std::cmp::Ordering::Equal => {
                let mut saved = persisted.next().unwrap();
                saved.text = live.next().unwrap().text;
                outline.push(saved);
            }
        }
    }
    outline.extend(persisted);
    outline.extend(live);
    outline
}

pub(crate) fn conversation_outline_target_is_loaded(
    items: &[ChatItem],
    user_offset: usize,
    target: usize,
) -> bool {
    let loaded = items
        .iter()
        .filter(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
        .count();
    (user_offset..user_offset + loaded).contains(&target)
}

/// Return a DOM-sized transcript slice without splitting a user turn. A
/// `requested_start` of `usize::MAX` follows the newest available turns.
pub(crate) fn transcript_render_window(
    items: &[ChatItem],
    requested_start: usize,
    max_user_turns: usize,
) -> (std::ops::Range<usize>, usize, usize) {
    let user_rows = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }).then_some(index)
        })
        .collect::<Vec<_>>();
    let total = user_rows.len();
    if total == 0 {
        return (0..items.len(), 0, 0);
    }
    let max_user_turns = max_user_turns.max(1);
    let latest_start = total.saturating_sub(max_user_turns);
    let start = if requested_start == usize::MAX {
        latest_start
    } else {
        requested_start.min(latest_start)
    };
    let end = (start + max_user_turns).min(total);
    let first_item = if start == 0 { 0 } else { user_rows[start] };
    let last_item = if end == total {
        items.len()
    } else {
        user_rows[end]
    };
    (first_item..last_item, start, total)
}

/// Find the prefix that can be unloaded after a completed live turn. The
/// durable transcript remains in SQLite; this only bounds the reactive rows
/// that otherwise grow for as long as the app stays open.
pub(crate) fn transcript_tail_trim_point(
    items: &[ChatItem],
    trim_above_user_turns: usize,
    retain_user_turns: usize,
) -> Option<(usize, usize)> {
    if items
        .iter()
        .any(|item| matches!(item, ChatItem::QueuedUser { .. }))
    {
        return None;
    }
    let user_rows = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| matches!(item, ChatItem::User(_)).then_some(index))
        .collect::<Vec<_>>();
    if user_rows.len() <= trim_above_user_turns {
        return None;
    }
    let dropped_turns = user_rows.len().saturating_sub(retain_user_turns.max(1));
    Some((user_rows[dropped_turns], dropped_turns))
}

#[cfg(test)]
mod transcript_render_window_tests {
    use super::{transcript_render_window, transcript_tail_trim_point};
    use crate::dto::ChatItem;

    #[test]
    fn limits_complete_user_turns_and_can_follow_the_tail() {
        let items = (0..6)
            .flat_map(|turn| {
                [
                    ChatItem::User(format!("question {turn}")),
                    ChatItem::Assistant {
                        text: format!("answer {turn}"),
                        model: None,
                        resources: Vec::new(),
                    },
                ]
            })
            .collect::<Vec<_>>();

        assert_eq!(transcript_render_window(&items, 0, 2), (0..4, 0, 6));
        assert_eq!(
            transcript_render_window(&items, usize::MAX, 2),
            (8..12, 4, 6)
        );
        assert_eq!(transcript_render_window(&items, 2, 2), (4..8, 2, 6));
    }

    #[test]
    fn trims_completed_live_history_in_page_sized_chunks() {
        let mut items = (0..5)
            .flat_map(|turn| {
                [
                    ChatItem::User(format!("question {turn}")),
                    ChatItem::Reasoning(format!("reasoning {turn}")),
                    ChatItem::Assistant {
                        text: format!("answer {turn}"),
                        model: None,
                        resources: Vec::new(),
                    },
                ]
            })
            .collect::<Vec<_>>();

        let (first_item, dropped_turns) = transcript_tail_trim_point(&items, 4, 2).unwrap();
        assert_eq!((first_item, dropped_turns), (9, 3));
        items.drain(..first_item);
        assert!(matches!(&items[0], ChatItem::User(text) if text == "question 3"));
        assert_eq!(transcript_tail_trim_point(&items, 4, 2), None);

        items.push(ChatItem::QueuedUser {
            id: 1,
            text: "queued".into(),
        });
        assert_eq!(transcript_tail_trim_point(&items, 1, 1), None);
    }
}

#[cfg(test)]
mod conversation_outline_tests {
    use super::{
        conversation_outline_target_is_loaded, merge_conversation_outline, owning_user_turn_index,
        queued_turn_rows, transcript_item_timestamp, turn_duration_ms, user_turn_index,
        QueuedTurnRow,
    };
    use crate::dto::{ChatItem, SessionOutlineItem};

    #[test]
    fn merges_live_turns_into_the_persisted_directory() {
        let persisted = vec![
            SessionOutlineItem {
                user_index: 0,
                seq: Some(1),
                text: "first".into(),
                sent_at: Some(100),
                response_at: Some(110),
            },
            SessionOutlineItem {
                user_index: 1,
                seq: Some(3),
                text: "stale second".into(),
                sent_at: Some(200),
                response_at: Some(210),
            },
        ];
        let items = vec![
            ChatItem::User("second".into()),
            ChatItem::Assistant {
                text: "answer".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser {
                id: 7,
                text: "third".into(),
            },
        ];

        assert_eq!(
            merge_conversation_outline(&persisted, &items, 1),
            vec![
                persisted[0].clone(),
                SessionOutlineItem {
                    user_index: 1,
                    seq: Some(3),
                    text: "second".into(),
                    sent_at: Some(200),
                    response_at: Some(210),
                },
                SessionOutlineItem {
                    user_index: 2,
                    seq: None,
                    text: "third".into(),
                    sent_at: None,
                    response_at: None,
                },
            ]
        );
        assert_eq!(
            transcript_item_timestamp(&items, 0, 1, &persisted),
            Some(200)
        );
        assert_eq!(
            transcript_item_timestamp(&items, 1, 1, &persisted),
            Some(210)
        );
        assert_eq!(user_turn_index(&items, 2), Some(1));
        assert_eq!(owning_user_turn_index(&items, 1), Some(0));
        assert_eq!(owning_user_turn_index(&items, 2), Some(1));
        assert!(conversation_outline_target_is_loaded(&items, 1, 2));
        assert!(!conversation_outline_target_is_loaded(&items, 1, 0));
        assert_eq!(
            queued_turn_rows(&items, 1),
            vec![QueuedTurnRow {
                id: 7,
                text: "third".into(),
                user_index: 2,
            }]
        );
    }

    #[test]
    fn composer_queue_rows_skip_sent_turns_and_keep_fifo_order() {
        let items = vec![
            ChatItem::User("already sent".into()),
            ChatItem::Assistant {
                text: "working".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser {
                id: 2,
                text: "first".into(),
            },
            ChatItem::QueuedUser {
                id: 3,
                text: "second".into(),
            },
        ];
        assert_eq!(
            queued_turn_rows(&items, 4),
            vec![
                QueuedTurnRow {
                    id: 2,
                    text: "first".into(),
                    user_index: 5,
                },
                QueuedTurnRow {
                    id: 3,
                    text: "second".into(),
                    user_index: 6,
                },
            ]
        );
        assert!(queued_turn_rows(&[ChatItem::User("only sent".into())], 0).is_empty());
    }

    #[test]
    fn turn_duration_uses_the_turn_boundaries() {
        assert_eq!(turn_duration_ms(Some(100), Some(480)), Some(380_000));
        assert_eq!(turn_duration_ms(Some(100), None), None);
        assert_eq!(turn_duration_ms(Some(100), Some(99)), None);
    }

    #[test]
    fn normalizes_an_old_unsorted_directory_before_merging() {
        let persisted = vec![
            SessionOutlineItem {
                user_index: 2,
                seq: Some(5),
                text: "third".into(),
                sent_at: None,
                response_at: None,
            },
            SessionOutlineItem {
                user_index: 0,
                seq: Some(1),
                text: "first".into(),
                sent_at: None,
                response_at: None,
            },
        ];

        let merged = merge_conversation_outline(&persisted, &[ChatItem::User("second".into())], 1);

        assert_eq!(
            merged
                .iter()
                .map(|entry| (entry.user_index, entry.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "first"), (1, "second"), (2, "third")]
        );
    }
}

pub(crate) fn focus_composer() {
    focus_element("composer-input");
}

pub(crate) fn focus_composer_at(caret: u32) {
    focus_composer();
    let Some(textarea) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("composer-input"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
    else {
        return;
    };
    let _ = textarea.set_selection_range(caret, caret);
}

pub(crate) fn focus_element(id: &str) {
    focus_element_inner(id, false);
}

pub(crate) fn focus_and_select_element(id: &str) {
    focus_element_inner(id, true);
}

fn focus_element_inner(id: &str, select_all: bool) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(el) = doc.get_element_by_id(id) else {
        return;
    };
    let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|e| e.focus());
    if !select_all {
        return;
    }
    if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        input.select();
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        ta.select();
    }
}

pub(crate) fn focus_element_soon(id: &'static str) {
    schedule_focus(id, false);
}

/// Focus a text field after the next paint and select its contents.
/// Used by rename/create modals so Ctrl/⌘A and typing work immediately.
pub(crate) fn focus_and_select_soon(id: &'static str) {
    schedule_focus(id, true);
}

fn schedule_focus(id: &'static str, select_all: bool) {
    let focus = Closure::once(move || {
        if select_all {
            focus_and_select_element(id);
        } else {
            focus_element(id);
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            focus.as_ref().unchecked_ref(),
            0,
        );
    }
    focus.forget();
}

pub(crate) fn attachment_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}
