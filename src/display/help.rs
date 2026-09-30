//! Custom help formatting for consistent CLI display.
//! This module provides functions to format help text and command descriptions

use crate::display::theme::Theme;
use console::style;

/// Format help text with consistent styling
pub fn format_help_section(title: &str, content: &str, indent: bool) -> String {
    let mut output = String::new();

    // Section header
    if Theme::should_disable_colors() {
        output.push_str(&format!("{title}\n"));
    } else {
        output.push_str(&format!("{}\n", style(title).cyan().bold()));
    }

    // Content with optional indentation
    for line in content.lines() {
        if line.trim().is_empty() {
            output.push('\n');
        } else if indent && !line.starts_with("    ") {
            output.push_str(&format!("    {line}\n"));
        } else {
            output.push_str(&format!("{line}\n"));
        }
    }

    output
}

/// Create styled help text for the CLI
pub fn create_help_text() -> String {
    let mut help = String::new();

    // Quick Start section
    let quick_start = r#"$ codanna init              # Initialize in current directory
$ codanna index src         # Index your source code
$ codanna serve --http --watch      # HTTP server with OAuth
$ codanna serve --https --watch     # HTTPS server with TLS"#;

    help.push_str(&format_help_section("QUICK START", quick_start, true));
    help.push('\n');

    // Learn More section
    let learn_more = r#"GitHub: https://github.com/bpstr/codanna"#;

    help.push_str(&format_help_section("LEARN MORE", learn_more, true));

    help
}

/// Format a command description with proper styling
pub fn format_command_description(name: &str, description: &str) -> String {
    if Theme::should_disable_colors() {
        format!("{name:16} {description}")
    } else {
        format!("{:16} {}", style(name).green(), description)
    }
}
