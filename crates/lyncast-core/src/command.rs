/// The built-in commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    ClipboardHistory,
    OpenConfig,
    ReloadApps,
    Quit,
    LockScreen,
    Suspend,
    LogOut,
    Reboot,
    PowerOff,
}

impl Command {
    /// Every command, in declaration order.
    // A new command needs an entry here too. The compiler checks every match, but not this list.
    pub const ALL: [Command; 9] = [
        Command::ClipboardHistory,
        Command::OpenConfig,
        Command::ReloadApps,
        Command::Quit,
        Command::LockScreen,
        Command::Suspend,
        Command::LogOut,
        Command::Reboot,
        Command::PowerOff,
    ];

    /// The title the row shows and the index matches.
    pub fn title(self) -> &'static str {
        match self {
            Command::ClipboardHistory => "Clipboard History",
            Command::OpenConfig => "Open Config",
            Command::ReloadApps => "Reload Apps",
            Command::Quit => "Quit lyncast",
            Command::LockScreen => "Lock Screen",
            Command::Suspend => "Suspend",
            Command::LogOut => "Log Out",
            Command::Reboot => "Reboot",
            Command::PowerOff => "Power Off",
        }
    }

    /// Words that make the command show without ranking it, as an app's `Keywords` do.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Command::ClipboardHistory => &["paste"],
            Command::OpenConfig => &["settings", "preferences"],
            Command::ReloadApps => &["refresh"],
            Command::Quit => &["exit"],
            Command::LockScreen => &[],
            Command::Suspend => &["sleep"],
            Command::LogOut => &["logout", "sign out"],
            Command::Reboot => &["restart"],
            Command::PowerOff => &["shutdown"],
        }
    }

    /// Log Out, Reboot, and Power Off ask before they run.
    pub fn confirms(self) -> bool {
        matches!(self, Command::LogOut | Command::Reboot | Command::PowerOff)
    }

    /// The id in a `cmd:` key.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Command::ClipboardHistory => "clipboard-history",
            Command::OpenConfig => "open-config",
            Command::ReloadApps => "reload-apps",
            Command::Quit => "quit",
            Command::LockScreen => "lock-screen",
            Command::Suspend => "suspend",
            Command::LogOut => "log-out",
            Command::Reboot => "reboot",
            Command::PowerOff => "power-off",
        }
    }
}
