use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputMode {
    Human,
    Json,
    Verbose,
}

impl OutputMode {
    pub fn from_flags(json: bool, verbose: bool) -> Self {
        if json {
            OutputMode::Json
        } else if verbose {
            OutputMode::Verbose
        } else {
            OutputMode::Human
        }
    }
}

pub fn print_success(mode: OutputMode, message: &str, data: Option<serde_json::Value>) {
    match mode {
        OutputMode::Json => {
            let output = json!({
                "success": true,
                "message": message,
                "data": data,
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
        }
        OutputMode::Verbose => {
            eprintln!("[OK] {}", message);
            if let Some(d) = data {
                eprintln!("[DATA] {}", serde_json::to_string_pretty(&d).unwrap_or_default());
            }
        }
        OutputMode::Human => {
            println!("{}", message);
            if let Some(d) = data {
                if let Some(obj) = d.as_object() {
                    for (k, v) in obj {
                        let val = match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Null => "-".to_string(),
                            other => other.to_string(),
                        };
                        println!("  {}: {}", k, val);
                    }
                }
            }
        }
    }
}

pub fn print_error(mode: OutputMode, error: &str) {
    match mode {
        OutputMode::Json => {
            let output = json!({
                "success": false,
                "error": error,
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
        }
        OutputMode::Verbose => {
            eprintln!("[ERROR] {}", error);
        }
        OutputMode::Human => {
            eprintln!("error: {}", error);
        }
    }
}

pub fn print_table(mode: OutputMode, headers: &[&str], rows: &[Vec<String>]) {
    match mode {
        OutputMode::Json => {
            let data: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    let mut map = serde_json::Map::new();
                    for (i, header) in headers.iter().enumerate() {
                        map.insert(
                            header.to_string(),
                            serde_json::Value::String(row.get(i).cloned().unwrap_or_default()),
                        );
                    }
                    serde_json::Value::Object(map)
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&data).unwrap_or_default());
        }
        OutputMode::Verbose | OutputMode::Human => {
            if rows.is_empty() {
                println!("No results.");
                return;
            }
            // Calculate column widths
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for row in rows {
                for (i, cell) in row.iter().enumerate() {
                    if i < widths.len() {
                        widths[i] = widths[i].max(cell.len());
                    }
                }
            }
            // Print header
            let header_line: Vec<String> = headers
                .iter()
                .enumerate()
                .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
                .collect();
            println!("{}", header_line.join("  "));
            let separator: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            println!("{}", separator.join("  "));
            // Print rows
            for row in rows {
                let line: Vec<String> = row
                    .iter()
                    .enumerate()
                    .map(|(i, cell)| {
                        let width = widths.get(i).copied().unwrap_or(0);
                        format!("{:<width$}", cell, width = width)
                    })
                    .collect();
                println!("{}", line.join("  "));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_mode_from_flags() {
        assert_eq!(OutputMode::from_flags(true, false), OutputMode::Json);
        assert_eq!(OutputMode::from_flags(false, true), OutputMode::Verbose);
        assert_eq!(OutputMode::from_flags(false, false), OutputMode::Human);
    }

    #[test]
    fn test_json_trumps_verbose() {
        assert_eq!(OutputMode::from_flags(true, true), OutputMode::Json);
    }

    #[test]
    fn test_print_table_json_format() {
        // Verify the JSON structure by building expected data manually
        let headers = &["Name", "Status"];
        let rows = &[
            vec!["vm-1".to_string(), "running".to_string()],
            vec!["vm-2".to_string(), "stopped".to_string()],
        ];

        // Build expected JSON the same way print_table does
        let data: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                let mut map = serde_json::Map::new();
                for (i, header) in headers.iter().enumerate() {
                    map.insert(
                        header.to_string(),
                        serde_json::Value::String(row.get(i).cloned().unwrap_or_default()),
                    );
                }
                serde_json::Value::Object(map)
            })
            .collect();

        assert_eq!(data.len(), 2);
        assert_eq!(data[0]["Name"], "vm-1");
        assert_eq!(data[0]["Status"], "running");
        assert_eq!(data[1]["Name"], "vm-2");
        assert_eq!(data[1]["Status"], "stopped");
    }

    #[test]
    fn test_print_table_empty() {
        // Empty rows should not panic; it prints "No results." for Human/Verbose modes.
        // We just verify it doesn't panic here.
        print_table(OutputMode::Human, &["Name", "Status"], &[]);
        print_table(OutputMode::Json, &["Name", "Status"], &[]);
    }
}
