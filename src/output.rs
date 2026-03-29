// Output helpers used by command handlers.

use comfy_table::{Cell, Table};
use serde::Serialize;

/// Print a value as JSON or as a human-readable table.
pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("error: failed to serialize value as JSON: {e}"),
    }
}

/// Print a simple key-value table.
pub fn print_kv(rows: &[(&str, &str)]) {
    let mut table = Table::new();
    table.load_preset(comfy_table::presets::UTF8_BORDERS_ONLY);
    for (k, v) in rows {
        table.add_row(vec![
            Cell::new(k).fg(comfy_table::Color::Cyan),
            Cell::new(v),
        ]);
    }
    println!("{table}");
}

/// Print a list table with headers.
pub fn print_table(headers: Vec<&str>, rows: Vec<Vec<String>>) {
    let mut table = Table::new();
    table.load_preset(comfy_table::presets::UTF8_FULL);
    table.set_header(
        headers
            .iter()
            .map(|h| Cell::new(h).fg(comfy_table::Color::Cyan)),
    );
    for row in rows {
        table.add_row(row);
    }
    println!("{table}");
}

/// Print a success message.
pub fn success(msg: &str) {
    println!("✓ {msg}");
}

/// Print an informational message.
pub fn info(msg: &str) {
    println!("{msg}");
}
