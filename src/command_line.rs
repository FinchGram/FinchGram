//! Terminal's command line (the design's 1b, "键盘优先 · / 命令行"; ui/terminal/commands.slint): the
//! rows of the list it shows above the line for what is typed. The commands themselves, what each
//! does and where it can run are the page's (CommandLine in ui/terminal/terminal.slint); the row
//! the arrow keys pick is a place in a sifted list, and a page cannot sift a list.

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{BotCommand, Command, Commands, MainWindow};

pub fn connect(ui: &MainWindow) {
    ui.global::<Commands>().on_matching(|commands, bots, typed| {
        let rows = rows(commands.iter().collect(), bots.iter().collect(), &typed);
        ModelRc::new(VecModel::from(rows))
    });
}

/// The rows for what is typed: the commands of the chat's bot that start so (the store has sifted
/// them already, here they are only told apart), then the line's own that can run now. A name
/// typed in full comes before the names it begins ("/me" before the bot's "/menu"): the first row
/// is the one Return runs.
fn rows(own: Vec<Command>, bots: Vec<BotCommand>, typed: &str) -> Vec<Command> {
    let name = typed.split(char::is_whitespace).next().unwrap_or_default().to_lowercase();
    let bots = bots
        .into_iter()
        .filter(|bot| offered(&bot.command.to_lowercase(), typed))
        .map(|bot| Command { command: bot.command, description: bot.description, available: true, words: false, bot: true });
    let own = own.into_iter().filter(|command| command.available && offered(&command.command, typed));
    let (exact, others): (Vec<Command>, Vec<Command>) = bots.chain(own).partition(|row| row.command.to_lowercase() == name);
    exact.into_iter().chain(others).collect()
}

/// Whether `command` is offered for what is typed: "/se" can still become "/search", in any case;
/// once a space follows the name, the name itself only ("/search keeb" is "/search", and no
/// longer "/settings").
fn offered(command: &str, typed: &str) -> bool {
    if !typed.starts_with('/') {
        return false;
    }
    let typed = typed.to_lowercase();
    match typed.split_once(char::is_whitespace) {
        Some((name, _)) => name == command,
        None => command.starts_with(typed.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::{offered, rows};
    use crate::{BotCommand, Command};

    fn own(command: &str, available: bool) -> Command {
        Command { command: command.into(), description: "".into(), available, words: false, bot: false }
    }

    fn bot(command: &str) -> BotCommand {
        BotCommand { command: command.into(), description: "".into() }
    }

    fn names(rows: Vec<Command>) -> Vec<String> {
        rows.into_iter().map(|row| format!("{}{}", if row.bot { "bot:" } else { "" }, row.command)).collect()
    }

    #[test]
    fn a_slash_alone_offers_every_command() {
        assert!(offered("/mute", "/"));
        assert!(offered("/settings", "/"));
    }

    #[test]
    fn the_start_of_a_name_offers_it() {
        assert!(offered("/search", "/se"));
        assert!(offered("/settings", "/se"));
        assert!(!offered("/mute", "/se"));
        assert!(offered("/search", "/search"));
        assert!(!offered("/me", "/mee"));
    }

    #[test]
    fn capitals_are_the_same_command() {
        assert!(offered("/tools", "/TO"));
    }

    #[test]
    fn words_after_the_name_leave_that_command_alone() {
        assert!(offered("/search", "/search keeb"));
        assert!(offered("/tools", "/tools 传输"));
        assert!(offered("/search", "/search "));
        assert!(!offered("/settings", "/se keeb"));
        assert!(!offered("/search", "/sea keeb"));
    }

    #[test]
    fn words_that_are_no_command_offer_nothing() {
        assert!(!offered("/search", "hello"));
        assert!(!offered("/search", ""));
    }

    #[test]
    fn the_bots_commands_come_before_the_lines_own() {
        let found = rows(vec![own("/search", true), own("/settings", true)], vec![bot("/start"), bot("/status")], "/s");
        assert_eq!(names(found), ["bot:/start", "bot:/status", "/search", "/settings"]);
    }

    #[test]
    fn a_command_that_cannot_run_here_is_not_offered() {
        let found = rows(vec![own("/photo", false), own("/pin", true)], Vec::new(), "/p");
        assert_eq!(names(found), ["/pin"]);
    }

    #[test]
    fn a_name_typed_in_full_is_the_first_row() {
        // "/me" is the profile, though the bot has a "/menu" that begins the same.
        let found = rows(vec![own("/me", true), own("/mute", true)], vec![bot("/menu")], "/me");
        assert_eq!(names(found), ["/me", "bot:/menu"]);
        // The bot's own command of the same name goes first: in its chat "/settings" is its.
        let found = rows(vec![own("/settings", true)], vec![bot("/settings")], "/settings");
        assert_eq!(names(found), ["bot:/settings", "/settings"]);
    }

    #[test]
    fn words_after_a_name_keep_its_row() {
        let found = rows(vec![own("/search", true), own("/settings", true)], Vec::new(), "/search keeb");
        assert_eq!(names(found), ["/search"]);
    }
}
