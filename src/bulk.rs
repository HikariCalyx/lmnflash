//! Bulk-IMEI lookup: the multi-line IMEI box, the sequential batch runner and
//! the CSV report it writes.
//!
//! The input state (`BulkInput`, `BulkRow`, `BulkStatus`) lives in the crate
//! root together with the other lookup inputs; this module owns the logic that
//! turns the box's lines into a report. One IMEI is looked up per task so the
//! UI can show real progress.

use iced::widget::text_editor;
use iced::Task;

use crate::{
    firmware, l10n, BulkRow, BulkStatus, LoginStatus, Message, State, BULK_STATUS_INVALID,
    BULK_STATUS_OK,
};

/// The header of the bulk-IMEI lookup CSV report.
const BULK_CSV_HEADER: &str =
    "imei,status,xtCode,carrier,build_fingerprint,download_link,lolinet_filename,assumed_directory";

/// The non-empty, trimmed lines of the bulk IMEI box.
pub(crate) fn bulk_lines(content: &text_editor::Content) -> Vec<String> {
    content
        .text()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Restricts the bulk IMEI box to digits and line breaks.
///
/// A typed character that is not a digit is dropped, and a pasted block is
/// reduced to its digits and line breaks (everything else disappears).
/// `None` means the action would change nothing and is not performed at all;
/// everything that is not an insertion (movement, selection, Backspace, …)
/// passes through untouched.
pub(crate) fn bulk_digits_only(action: text_editor::Action) -> Option<text_editor::Action> {
    use text_editor::{Action, Edit};

    match action {
        Action::Edit(Edit::Insert(character)) => {
            character
                .is_ascii_digit()
                .then_some(Action::Edit(Edit::Insert(character)))
        }
        Action::Edit(Edit::Paste(text)) => {
            let filtered: String = text
                .chars()
                .filter(|character| character.is_ascii_digit() || *character == '\n')
                .collect();

            (!filtered.is_empty())
                .then(|| Action::Edit(Edit::Paste(std::sync::Arc::new(filtered))))
        }
        action => Some(action),
    }
}

/// Validates the bulk IMEI box and queues every line for lookup.
pub(crate) fn start_bulk_lookup(state: &mut State) -> Task<Message> {
    if !matches!(state.login, LoginStatus::LoggedIn { .. }) {
        return Task::none();
    }

    let lines = bulk_lines(&state.lookup.bulk.content);
    if lines.is_empty() {
        state.lookup.bulk.status = BulkStatus::Error(state.l10n.tr("bulk-error-empty"));
        state.lookup.bulk.save_note = None;
        return Task::none();
    }

    let total = lines.len();
    {
        let bulk = &mut state.lookup.bulk;
        bulk.queue = lines;
        bulk.index = 0;
        bulk.rows.clear();
        bulk.save_note = None;
        bulk.status = BulkStatus::Running { current: 0, total };
    }

    bulk_next(state)
}

/// Looks up the next queued IMEI, skipping invalid lines, or finishes the
/// batch (and asks where to save the CSV).
///
/// One IMEI is looked up per task so the UI can show real progress; the loop
/// below only spins over invalid lines, which never reach the server.
pub(crate) fn bulk_next(state: &mut State) -> Task<Message> {
    let token = match &state.login {
        LoginStatus::LoggedIn { token, .. } => token.clone(),
        _ => return Task::none(),
    };

    let uuid = state.client_uuid.clone();

    loop {
        let index = state.lookup.bulk.index;
        let total = state.lookup.bulk.queue.len();

        if index >= total {
            state.lookup.bulk.status = BulkStatus::Done;
            return start_bulk_save(state);
        }

        let line = state.lookup.bulk.queue[index].clone();

        match firmware::validate_imei(&line) {
            Ok(imei) => {
                state.lookup.bulk.status = BulkStatus::Running {
                    current: index + 1,
                    total,
                };

                let token = token.clone();
                let uuid = uuid.clone();

                return Task::perform(
                    async move {
                        let request_imei = imei.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            firmware::fetch_firmware(&request_imei, &token, &uuid)
                        })
                        .await
                        .unwrap_or_else(|error| {
                            Err(firmware::FirmwareError::Other(format!(
                                "background task failed: {error}"
                            )))
                        });

                        (imei, result)
                    },
                    |(imei, result)| Message::BulkStepFinished(imei, result),
                );
            }
            Err(_) => {
                // An invalid line never reaches the server: it is reported as
                // "invalid imei" and the batch moves on.
                state
                    .lookup
                    .bulk
                    .rows
                    .push(BulkRow::skipped(&line, BULK_STATUS_INVALID));
                state.lookup.bulk.index += 1;
            }
        }
    }
}

/// Asks for a file name and writes the collected rows there as CSV.
pub(crate) fn start_bulk_save(state: &mut State) -> Task<Message> {
    if state.lookup.bulk.rows.is_empty() {
        return Task::none();
    }

    let csv = bulk_csv(&state.lookup.bulk.rows);
    state.lookup.bulk.save_note = None;

    // The save dialog blocks, so it runs on a worker thread (like the flash
    // script export); the file is written there too.
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let Some(path) = rfd::FileDialog::new()
                    .set_file_name("imei_lookup.csv")
                    .add_filter("CSV", &["csv"])
                    .save_file()
                else {
                    return Ok(None);
                };

                std::fs::write(&path, csv)
                    .map_err(|error| format!("{}: {error}", path.display()))?;

                Ok(Some(path))
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        Message::BulkSaveFinished,
    )
}

/// Renders the bulk-lookup rows as a CSV document (RFC 4180 line endings).
fn bulk_csv(rows: &[BulkRow]) -> String {
    let mut csv = String::from(BULK_CSV_HEADER);
    csv.push_str("\r\n");

    for row in rows {
        let fields = [
            row.imei.as_str(),
            row.status,
            row.xt_code.as_str(),
            row.carrier.as_str(),
            row.build_fingerprint.as_str(),
            row.download_link.as_str(),
            row.lolinet_filename.as_str(),
            row.assumed_directory.as_str(),
        ];

        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                csv.push(',');
            }
            csv.push_str(&csv_field(field));
        }

        csv.push_str("\r\n");
    }

    csv
}

/// Quotes a CSV field when it holds a comma, a quote or a line break.
fn csv_field(value: &str) -> String {
    if value
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\n' | '\r'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// The "N IMEIs looked up: …" line of a finished batch.
pub(crate) fn bulk_summary(l10n: &l10n::Bundle, rows: &[BulkRow]) -> String {
    let count = |status: &str| rows.iter().filter(|row| row.status == status).count();

    let total = rows.len();
    let found = count(BULK_STATUS_OK);
    let invalid = count(BULK_STATUS_INVALID);
    // Everything that is neither found nor invalid produced no firmware
    // (the server had none, or the request failed).
    let failed = total - found - invalid;

    l10n.tr_with_args(
        "bulk-summary",
        &[
            ("total", total.to_string()),
            ("found", found.to_string()),
            ("invalid", invalid.to_string()),
            ("failed", failed.to_string()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BULK_STATUS_FAILED, BULK_STATUS_MISSING};

    #[test]
    fn bulk_lines_skips_blank_lines() {
        let content = text_editor::Content::with_text("  350000000000001  \n\n350000000000002\n");

        assert_eq!(
            bulk_lines(&content),
            vec!["350000000000001".to_string(), "350000000000002".to_string()]
        );
    }

    /// The bulk IMEI box only accepts digits and line breaks: a typed
    /// non-digit is dropped, a paste is reduced to what is usable and a paste
    /// with nothing usable is not performed at all.
    #[test]
    fn bulk_editor_keeps_only_digits_and_line_breaks() {
        use iced::widget::text_editor::{Action, Edit};
        use std::sync::Arc;

        assert!(bulk_digits_only(Action::Edit(Edit::Insert('5'))).is_some());
        assert!(bulk_digits_only(Action::Edit(Edit::Insert('a'))).is_none());
        assert!(bulk_digits_only(Action::Edit(Edit::Insert(' '))).is_none());

        assert!(matches!(
            bulk_digits_only(Action::Edit(Edit::Enter)),
            Some(Action::Edit(Edit::Enter))
        ));
        assert!(bulk_digits_only(Action::Move(text_editor::Motion::Right)).is_some());

        match bulk_digits_only(Action::Edit(Edit::Paste(Arc::new("12a-3\n45 x".to_string())))) {
            Some(Action::Edit(Edit::Paste(text))) => assert_eq!(text.as_str(), "123\n45"),
            other => panic!("expected a filtered paste, got {other:?}"),
        }

        assert!(bulk_digits_only(Action::Edit(Edit::Paste(Arc::new("abc".to_string())))).is_none());
    }

    #[test]
    fn bulk_csv_writes_the_header_and_rows() {
        let rows = vec![
            BulkRow {
                imei: "350000000000001".to_string(),
                status: BULK_STATUS_OK,
                xt_code: "XT2507-4".to_string(),
                carrier: "retin".to_string(),
                build_fingerprint: "motorola/cybert/cybert:17/U1TQS34.28-11".to_string(),
                download_link: "https://example.com/a.zip?x=1".to_string(),
                lolinet_filename: "XT2507-4_CYBERT_RETIN_17_A.zip".to_string(),
                assumed_directory: "2025/cybert/official/RETIN".to_string(),
            },
            BulkRow::skipped("not-a-number", BULK_STATUS_INVALID),
        ];

        assert_eq!(
            bulk_csv(&rows),
            "imei,status,xtCode,carrier,build_fingerprint,download_link,lolinet_filename,assumed_directory\r\n\
             350000000000001,ok,XT2507-4,retin,motorola/cybert/cybert:17/U1TQS34.28-11,https://example.com/a.zip?x=1,XT2507-4_CYBERT_RETIN_17_A.zip,2025/cybert/official/RETIN\r\n\
             not-a-number,invalid imei,,,,,,\r\n"
        );
    }

    #[test]
    fn csv_fields_are_quoted_when_needed() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
    }

    #[test]
    fn bulk_summary_counts_each_status() {
        let bundle = l10n::bundle_for(l10n::Language::EnUs);
        let rows = [
            BulkRow::skipped("a", BULK_STATUS_OK),
            BulkRow::skipped("b", BULK_STATUS_MISSING),
            BulkRow::skipped("c", BULK_STATUS_INVALID),
            BulkRow::skipped("d", BULK_STATUS_FAILED),
        ];

        assert_eq!(
            bulk_summary(&bundle, &rows),
            "4 IMEIs looked up: 1 found, 1 invalid, 2 failed"
        );
    }
}
