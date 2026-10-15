//! A key's CLI and socket spelling: what parses, what prints, and what is refused.

use lyncast_core::{AppId, ClipId, Command, Key, KeyError, Kind};

#[test]
fn keys_parse_from_their_spelling() {
    let cases = [
        ("app:firefox", Key::App(AppId("firefox".into()))),
        ("cmd:reboot", Key::Command(Command::Reboot)),
        ("cmd:power-off", Key::Command(Command::PowerOff)),
        ("cmd:quit", Key::Command(Command::Quit)),
        ("clip:7", Key::Clip(ClipId(7))),
        ("clip:007", Key::Clip(ClipId(7))),
        ("calc:a:b", Key::Calc("a:b".into())),
        ("app:a:b", Key::App(AppId("a:b".into()))),
        ("clip:9223372036854775807", Key::Clip(ClipId(i64::MAX))),
    ];
    for (text, key) in cases {
        assert_eq!(text.parse::<Key>(), Ok(key), "{text}");
    }
}

#[test]
fn keys_print_their_spelling() {
    let cases = [
        (Key::Calc("a:b".into()), "calc:a:b"),
        (Key::Command(Command::PowerOff), "cmd:power-off"),
        (Key::Clip(ClipId(7)), "clip:7"),
        (
            Key::App(AppId("org.gnome.Nautilus".into())),
            "app:org.gnome.Nautilus",
        ),
    ];
    for (key, text) in cases {
        assert_eq!(key.to_string(), text);
    }
}

#[test]
fn malformed_keys_are_refused() {
    let cases = [
        ("firefox", KeyError::UnknownKind),
        ("file:/etc/hosts", KeyError::UnknownKind),
        ("APP:firefox", KeyError::UnknownKind),
        ("apps:firefox", KeyError::UnknownKind),
        ("app:", KeyError::Empty),
        ("calc:", KeyError::Empty),
        ("cmd:frobnicate", KeyError::UnknownCommand),
        ("cmd:", KeyError::UnknownCommand),
        ("cmd:Reboot", KeyError::UnknownCommand),
        ("cmd:reboot-now", KeyError::UnknownCommand),
        ("clip:abc", KeyError::BadClipId),
        ("clip:+7", KeyError::BadClipId),
        ("clip:-1", KeyError::BadClipId),
        ("clip:7 ", KeyError::BadClipId),
        ("clip:", KeyError::BadClipId),
        ("clip:9223372036854775808", KeyError::BadClipId),
    ];
    for (text, error) in cases {
        assert_eq!(text.parse::<Key>(), Err(error), "{text}");
    }
}

#[test]
fn a_parsed_key_knows_its_kind() {
    assert_eq!(
        "calc:2+2".parse::<Key>().map(|key| key.kind()),
        Ok(Kind::Calculator)
    );
    assert_eq!(
        "cmd:suspend".parse::<Key>().map(|key| key.kind()),
        Ok(Kind::Command)
    );
}

/// Two commands sharing an id would make the second parse as the first, and a command
/// listed twice in `ALL` hides another. Ids are kebab-case, per the decision log.
#[test]
fn every_command_key_is_kebab_case_and_parses_back_to_itself() {
    for command in Command::ALL {
        let listed = Command::ALL.iter().filter(|c| **c == command).count();
        assert_eq!(listed, 1, "{command:?}");
        let key = Key::Command(command);
        let text = key.to_string();
        let id = &text["cmd:".len()..];
        let kebab = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
        assert!(!id.is_empty() && id.bytes().all(kebab), "{text}");
        assert_eq!(text.parse::<Key>(), Ok(key), "{text}");
    }
}
