//! The chord table and a subject's key, called the way the palette and the daemon call them.

use lyncast_core::{
    ActionItem, Actions, AppId, Chord, ClipId, Command, DesktopAction, Key, Subject, Verb,
};

fn firefox_id() -> AppId {
    AppId("firefox".into())
}

fn new_window() -> DesktopAction {
    DesktopAction {
        id: "new-window".into(),
        name: "New Window".into(),
    }
}

fn new_private_window() -> DesktopAction {
    DesktopAction {
        id: "new-private-window".into(),
        name: "New Private Window".into(),
    }
}

fn firefox() -> Subject {
    Subject::App {
        id: firefox_id(),
        actions: vec![new_window(), new_private_window()],
    }
}

fn clip(pinned: bool) -> Subject {
    Subject::Clip {
        id: ClipId(7),
        pinned,
    }
}

fn calc() -> Subject {
    Subject::Calc {
        query: "2+3*4".into(),
    }
}

/// The Actions menu as the palette draws it and the daemon runs it: the key hint, the
/// label, the verb a pick sends, and the item that runs.
fn menu(actions: &Actions) -> Vec<(Option<Chord>, &str, Verb, ActionItem)> {
    actions
        .iter()
        .map(|(chord, item)| (chord, item.label(), item.verb(), item.clone()))
        .collect()
}

#[test]
fn an_app_offers_open_then_its_desktop_actions_then_copy_path() {
    assert_eq!(
        menu(&firefox().actions()),
        [
            (
                Some(Chord::Enter),
                "Open",
                Verb::Launch,
                ActionItem::Launch(firefox_id())
            ),
            (
                None,
                "New Window",
                Verb::LaunchAction("new-window".into()),
                ActionItem::LaunchAction(firefox_id(), new_window())
            ),
            (
                None,
                "New Private Window",
                Verb::LaunchAction("new-private-window".into()),
                ActionItem::LaunchAction(firefox_id(), new_private_window())
            ),
            (
                None,
                "Copy Path to .desktop",
                Verb::CopyPath,
                ActionItem::CopyPath(firefox_id())
            ),
        ]
    );
}

#[test]
fn an_app_without_desktop_actions_still_offers_copy_path() {
    let nautilus = AppId("org.gnome.Nautilus".into());
    let subject = Subject::App {
        id: nautilus.clone(),
        actions: Vec::new(),
    };
    assert_eq!(
        menu(&subject.actions()),
        [
            (
                Some(Chord::Enter),
                "Open",
                Verb::Launch,
                ActionItem::Launch(nautilus.clone())
            ),
            (
                None,
                "Copy Path to .desktop",
                Verb::CopyPath,
                ActionItem::CopyPath(nautilus)
            ),
        ]
    );
}

#[test]
fn an_unpinned_clip_offers_pin_and_a_pinned_one_offers_unpin() {
    assert_eq!(
        menu(&clip(false).actions()),
        [
            (
                Some(Chord::Enter),
                "Copy",
                Verb::Copy,
                ActionItem::CopyClip(ClipId(7))
            ),
            (None, "Pin", Verb::Pin, ActionItem::Pin(ClipId(7))),
            (None, "Delete", Verb::Delete, ActionItem::Delete(ClipId(7))),
        ]
    );
    assert_eq!(
        menu(&clip(true).actions()),
        [
            (
                Some(Chord::Enter),
                "Copy",
                Verb::Copy,
                ActionItem::CopyClip(ClipId(7))
            ),
            (None, "Unpin", Verb::Unpin, ActionItem::Unpin(ClipId(7))),
            (None, "Delete", Verb::Delete, ActionItem::Delete(ClipId(7))),
        ]
    );
}

#[test]
fn the_calculator_card_copies_the_answer_to_its_raw_query() {
    assert_eq!(
        menu(&calc().actions()),
        [(
            Some(Chord::Enter),
            "Copy Answer",
            Verb::Copy,
            ActionItem::CopyAnswer("2+3*4".into())
        )]
    );
}

#[test]
fn a_command_offers_run() {
    assert_eq!(
        menu(&Subject::Command(Command::Reboot).actions()),
        [(
            Some(Chord::Enter),
            "Run",
            Verb::Run,
            ActionItem::Run(Command::Reboot)
        )]
    );
}

#[test]
fn enter_and_ctrl_enter_both_run_the_default_on_every_kind() {
    let cases = [
        (firefox(), ActionItem::Launch(firefox_id())),
        (clip(true), ActionItem::CopyClip(ClipId(7))),
        (calc(), ActionItem::CopyAnswer("2+3*4".into())),
        (
            Subject::Command(Command::Reboot),
            ActionItem::Run(Command::Reboot),
        ),
        (
            Subject::Command(Command::LogOut),
            ActionItem::Run(Command::LogOut),
        ),
    ];
    for (subject, default) in cases {
        let actions = subject.actions();
        assert_eq!(actions.on(Chord::Enter), &default);
        assert_eq!(actions.on(Chord::CtrlEnter), &default);
    }
}

#[test]
fn a_menu_pick_runs_only_what_the_row_offers_now() {
    assert_eq!(
        clip(false).actions().pick(&Verb::Pin),
        Some(&ActionItem::Pin(ClipId(7)))
    );
    // A menu drawn before the clip was pinned.
    assert_eq!(clip(true).actions().pick(&Verb::Pin), None);
    assert_eq!(
        firefox()
            .actions()
            .pick(&Verb::LaunchAction("new-private-window".into())),
        Some(&ActionItem::LaunchAction(
            firefox_id(),
            new_private_window()
        ))
    );
    assert_eq!(
        firefox()
            .actions()
            .pick(&Verb::LaunchAction("profile-manager".into())),
        None
    );
    assert_eq!(
        calc().actions().pick(&Verb::Copy),
        Some(&ActionItem::CopyAnswer("2+3*4".into()))
    );
    assert_eq!(calc().actions().pick(&Verb::Delete), None);
}

/// A row whose menu showed an item it then refused to run would strand the palette.
#[test]
fn every_item_a_menu_shows_runs_when_picked() {
    for subject in [
        firefox(),
        clip(false),
        clip(true),
        calc(),
        Subject::Command(Command::LogOut),
    ] {
        let actions = subject.actions();
        for (_, item) in actions.iter() {
            assert_eq!(actions.pick(&item.verb()), Some(item), "{subject:?}");
        }
    }
}

#[test]
fn a_subject_keys_by_its_id_alone() {
    assert_eq!(clip(true).key(), Key::Clip(ClipId(7)));
    assert_eq!(clip(false).key(), Key::Clip(ClipId(7)));
    assert_eq!(firefox().key(), Key::App(firefox_id()));
    assert_eq!(
        Subject::App {
            id: AppId("org.gnome.Nautilus".into()),
            actions: Vec::new(),
        }
        .key(),
        Key::App(AppId("org.gnome.Nautilus".into()))
    );
    assert_eq!(calc().key(), Key::Calc("2+3*4".into()));
    assert_eq!(
        Subject::Clip {
            id: ClipId(8),
            pinned: false
        }
        .key(),
        Key::Clip(ClipId(8))
    );
    assert_eq!(
        Subject::Command(Command::LogOut).key(),
        Key::Command(Command::LogOut)
    );
}
