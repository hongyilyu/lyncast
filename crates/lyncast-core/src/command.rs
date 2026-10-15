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
