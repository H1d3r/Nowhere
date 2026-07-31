use super::*;
use crate::tui::model::{FeedKind, Focus, InstanceMeta, InstanceRole, Lifecycle, Page, UiEvent};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn edits_filter_and_returns_to_navigation() {
    let mut app = App::default();
    app.page = Page::Logs;
    handle_key(&mut app, key(KeyCode::Char('/')));
    handle_key(&mut app, key(KeyCode::Char('q')));
    handle_key(&mut app, key(KeyCode::Char('u')));
    assert_eq!(app.filter, "qu");
    assert!(!app.should_quit);
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(&mut app, key(KeyCode::Char('q')));
    assert!(app.should_quit);
}

#[test]
fn navigates_instances_without_wrapping() {
    let mut app = App::default();
    for pid in [2, 1] {
        app.apply(UiEvent::Upsert {
            meta: InstanceMeta {
                id: pid.to_string(),
                role: InstanceRole::Portal,
                pid,
                uid: 0,
                endpoint: format!(":{pid}"),
                ..InstanceMeta::default()
            },
            lifecycle: Lifecycle::Ready,
            snapshot: None,
        });
    }
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(app.selected().unwrap().meta.pid, 2);
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(app.selected().unwrap().meta.pid, 2);
    handle_key(&mut app, key(KeyCode::Up));
    assert_eq!(app.selected().unwrap().meta.pid, 1);
}

#[test]
fn pause_and_resume_return_to_live_tail() {
    let mut app = App::default();
    app.page = Page::Logs;
    app.feed_scroll = 12;
    app.paused = true;
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(!app.paused);
    assert_eq!(app.feed_scroll, 0);
}

#[test]
fn tab_cycles_through_both_logs_and_back_to_instances() {
    let mut app = App::default();
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.page, Page::Logs);
    assert_eq!(app.feed, FeedKind::Access);
    assert_eq!(app.focus, Focus::Feed);

    handle_key(&mut app, key(KeyCode::Right));
    assert_eq!(app.access_horizontal_scroll, 4);

    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.feed, FeedKind::Runtime);
    assert_eq!(app.focus, Focus::Feed);
    handle_key(&mut app, key(KeyCode::Right));
    assert_eq!(app.runtime_horizontal_scroll, 4);

    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Instances);
    assert_eq!(app.page, Page::Logs);

    handle_key(&mut app, key(KeyCode::BackTab));
    assert_eq!(app.feed, FeedKind::Runtime);
    assert_eq!(app.focus, Focus::Feed);
}

#[test]
fn number_keys_switch_between_the_two_workspaces() {
    let mut app = App::default();
    handle_key(&mut app, key(KeyCode::Char('2')));
    assert_eq!(app.page, Page::Logs);
    assert_eq!(app.focus, Focus::Instances);

    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Feed);
    handle_key(&mut app, key(KeyCode::Char('1')));
    assert_eq!(app.page, Page::Overview);
    assert_eq!(app.focus, Focus::Instances);
}

#[test]
fn log_only_commands_do_not_change_overview_state() {
    let mut app = App::default();
    app.feed_scroll = 12;
    assert!(!handle_key(&mut app, key(KeyCode::Char(' '))));
    assert!(!handle_key(&mut app, key(KeyCode::Char('/'))));
    assert!(!handle_key(&mut app, key(KeyCode::PageUp)));
    assert!(!app.paused);
    assert!(!app.filter_editing);
    assert_eq!(app.feed_scroll, 12);
}

#[test]
fn legacy_log_shortcuts_are_unbound() {
    let mut app = App::default();
    assert!(!handle_key(&mut app, key(KeyCode::Char('a'))));
    assert!(!handle_key(&mut app, key(KeyCode::Char('r'))));
    assert!(!handle_key(&mut app, key(KeyCode::Char('3'))));
    assert!(!handle_key(&mut app, key(KeyCode::Char('4'))));
    assert_eq!(app.focus, Focus::Instances);
}
