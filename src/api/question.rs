//! `agent.question` and `agent.answer`: the question a blocked agent shows,
//! as structured options, and answering it the way a person would (arrow
//! keys, space and Enter, or the option's own key).
//!
//! Agents do not hand their questions to Herdr, so the options are read
//! from the bottom of the agent's screen: numbered lists (`❯ 1. Yes`),
//! keyed lists (`[a] Trust this workspace`) and checkbox lists. Only the
//! last block of options is used, never older text higher up.

use crate::api::schema::{
    AgentAnswerParams, AgentQuestion, AgentQuestionOption, AgentStatus, Method, ReadFormat,
    ReadSource, Request, ResponseResult,
};
use crate::api::server::{dispatch_to_app_with_timeout, APP_RESPONSE_TIMEOUT};
use crate::api::ApiRequestSender;

type QuestionError = (&'static str, String);

/// Lines from the bottom of the screen that a question may use.
const QUESTION_LINES: usize = 40;

pub(super) fn question(
    request_id: &str,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, QuestionError> {
    let (status, screen) = read_agent(request_id, pane_id, api_tx)?;
    let question = (status == AgentStatus::Blocked)
        .then(|| parse_question(&screen))
        .flatten();
    Ok(ResponseResult::AgentQuestion { question })
}

pub(super) fn answer(
    request_id: &str,
    params: AgentAnswerParams,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, QuestionError> {
    let (status, screen) = read_agent(request_id, &params.pane_id, api_tx)?;
    if status != AgentStatus::Blocked {
        return Err(("no_question", "the agent is not asking anything".into()));
    }
    let question = parse_question(&screen).ok_or((
        "no_question",
        "no options found on the agent's screen".to_string(),
    ))?;
    let keys = answer_keys(&question, &params)?;
    for chunk in keys {
        match chunk {
            AnswerInput::Keys(keys) => app(
                request_id,
                Method::PaneSendKeys(crate::api::schema::PaneSendKeysParams {
                    pane_id: params.pane_id.clone(),
                    keys,
                }),
                api_tx,
            )?,
            AnswerInput::Text(text) => app(
                request_id,
                Method::PaneSendText(crate::api::schema::PaneSendTextParams {
                    pane_id: params.pane_id.clone(),
                    text,
                }),
                api_tx,
            )?,
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(ResponseResult::AgentAnswered { question })
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum AnswerInput {
    Keys(Vec<String>),
    Text(String),
}

/// The input that picks `params`' answer in `question`.
pub(super) fn answer_keys(
    question: &AgentQuestion,
    params: &AgentAnswerParams,
) -> Result<Vec<AnswerInput>, QuestionError> {
    let find = |wanted: &str| {
        question
            .options
            .iter()
            .position(|option| option.key.as_deref() == Some(wanted) || option.label == wanted)
            .ok_or_else(|| ("unknown_option", format!("no option {wanted:?}")))
    };
    let current = question
        .options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    let move_to = |from: usize, to: usize| {
        let key = if to > from { "down" } else { "up" };
        vec![key.to_string(); from.abs_diff(to)]
    };

    if let Some(text) = params.text.as_deref() {
        if !question.free_text {
            return Err((
                "free_text_not_allowed",
                "this question takes one of its options".into(),
            ));
        }
        let mut inputs = Vec::new();
        // Agents offer free text as an option ("Type something"); pick it first.
        if let Some(index) = question.options.iter().position(is_free_text_option) {
            let mut keys = move_to(current, index);
            keys.push("enter".into());
            inputs.push(AnswerInput::Keys(keys));
        }
        inputs.push(AnswerInput::Text(text.to_string()));
        inputs.push(AnswerInput::Keys(vec!["enter".into()]));
        return Ok(inputs);
    }
    if !params.options.is_empty() {
        if !question.multi_select {
            return Err((
                "not_multi_select",
                "this question takes a single option".into(),
            ));
        }
        let mut keys = Vec::new();
        let mut at = current;
        let mut targets = params
            .options
            .iter()
            .map(|wanted| find(wanted))
            .collect::<Result<Vec<_>, _>>()?;
        targets.sort_unstable();
        for target in targets {
            keys.extend(move_to(at, target));
            if !question.options[target].checked {
                keys.push("space".into());
            }
            at = target;
        }
        keys.push("enter".into());
        return Ok(vec![AnswerInput::Keys(keys)]);
    }
    let wanted = params
        .option
        .as_deref()
        .ok_or(("invalid_params", "give option, options or text".to_string()))?;
    let index = find(wanted)?;
    let option = &question.options[index];
    // A letter key picks its option directly ([a] Trust); numbered lists are
    // walked with the arrows, which every list picker accepts.
    if option
        .key
        .as_deref()
        .is_some_and(|key| key.chars().all(char::is_alphabetic))
    {
        return Ok(vec![AnswerInput::Keys(vec![option
            .key
            .clone()
            .unwrap_or_default()])]);
    }
    let mut keys = move_to(current, index);
    keys.push("enter".into());
    Ok(vec![AnswerInput::Keys(keys)])
}

fn is_free_text_option(option: &AgentQuestionOption) -> bool {
    let label = option.label.to_lowercase();
    label.starts_with("type something")
        || label.starts_with("type your own")
        || label.starts_with("other")
        // "No, and tell Claude what to do differently" opens a text box.
        || (label.contains("tell ") && label.contains("what to do"))
}

/// Parses the last block of options on screen and the question text above it.
pub(super) fn parse_question(screen: &str) -> Option<AgentQuestion> {
    let lines: Vec<&str> = screen.lines().collect();
    let start = lines.len().saturating_sub(QUESTION_LINES);
    let lines: Vec<String> = lines[start..]
        .iter()
        .map(|line| strip_box(line).to_string())
        .collect();

    let mut last_option = None;
    for (index, line) in lines.iter().enumerate().rev() {
        if parse_option(line).is_some() {
            last_option = Some(index);
            break;
        }
    }
    let Some(end) = last_option else {
        return parse_marked_list(&lines);
    };
    let mut first = end;
    while first > 0 {
        let previous = &lines[first - 1];
        if parse_option(previous).is_some() {
            first -= 1;
        } else if previous.trim().is_empty() || is_description(previous) {
            // Options may have a description line or a gap between them.
            if (first.saturating_sub(3)..first - 1).any(|i| parse_option(&lines[i]).is_some()) {
                first -= 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    let mut options = Vec::new();
    for line in &lines[first..=end] {
        if let Some(option) = parse_option(line) {
            options.push(option);
        } else if is_description(line) {
            if let Some(option) = options.last_mut() {
                let text = line.trim().to_string();
                option.description = Some(match option.description.take() {
                    Some(previous) => format!("{previous} {text}"),
                    None => text,
                });
            }
        }
    }
    if options.len() < 2 {
        return None;
    }
    let text = question_text(&lines[..first]);
    let multi_select = options.iter().any(|option| option.checkbox);
    let free_text = options.iter().any(is_free_text_option);
    Some(AgentQuestion {
        text,
        options,
        multi_select,
        free_text,
        source: "screen".into(),
    })
}

/// The question above an option block: from the nearest line ending in `?`
/// within a few lines (skipping details such as a folder path), else the
/// paragraph right above the options.
fn question_text(above: &[String]) -> String {
    let lines: Vec<&str> = above.iter().map(|line| line.trim()).collect();
    // The nearest line asking something within a few lines, read with the
    // rest of its paragraph (skipping details such as a folder path).
    let mut question = None;
    for (index, line) in lines.iter().enumerate().rev().take(10) {
        if is_rule(line) {
            break;
        }
        if line.contains('?') {
            question = Some(index);
            break;
        }
    }
    if let Some(index) = question {
        let mut start = index;
        while start > 0
            && !lines[start - 1].is_empty()
            && !is_rule(lines[start - 1])
            && index - start < 2
        {
            start -= 1;
        }
        let end = (index..lines.len())
            .take_while(|at| !lines[*at].is_empty() && !is_rule(lines[*at]))
            .last()
            .unwrap_or(index)
            .min(index + 3);
        return lines[start..=end].join("\n");
    }
    let mut paragraph = Vec::new();
    for line in lines.iter().rev() {
        if line.is_empty() {
            if paragraph.is_empty() {
                continue;
            }
            break;
        }
        if is_rule(line) {
            break;
        }
        paragraph.push(*line);
        if paragraph.len() >= 6 {
            break;
        }
    }
    paragraph.reverse();
    paragraph.join("\n")
}

/// A list without numbers or keys: the line with the cursor marker
/// (`❯ No, exit`) and the lines next to it that start at the same column.
fn parse_marked_list(lines: &[String]) -> Option<AgentQuestion> {
    let markers = ["❯", "›", "▶", "→"];
    let (marked, column) = lines.iter().enumerate().rev().find_map(|(index, line)| {
        let indent = line.len() - line.trim_start().len();
        let rest = line.trim_start();
        let marker = markers.iter().find(|marker| rest.starts_with(**marker))?;
        let after = &rest[marker.len()..];
        // Columns in characters: a marker such as `❯` is several bytes.
        let text_start = indent
            + marker.chars().count()
            + (after.chars().count() - after.trim_start().chars().count());
        (!after.trim().is_empty()).then_some((index, text_start))
    })?;
    let at_column = |line: &String| {
        let indent = line.len() - line.trim_start().len();
        indent == column && !line.trim().is_empty()
    };
    let mut first = marked;
    while first > 0 && at_column(&lines[first - 1]) {
        first -= 1;
    }
    let mut last = marked;
    while last + 1 < lines.len() && at_column(&lines[last + 1]) {
        last += 1;
    }
    if last == first {
        return None;
    }
    let options: Vec<AgentQuestionOption> = (first..=last)
        .map(|index| {
            let line = lines[index].trim_start();
            let selected = index == marked;
            let label = if selected {
                let marker = markers
                    .iter()
                    .find(|marker| line.starts_with(**marker))
                    .unwrap_or(&"");
                line[marker.len()..].trim()
            } else {
                line.trim()
            };
            AgentQuestionOption {
                label: label.to_string(),
                key: None,
                description: None,
                selected,
                checkbox: false,
                checked: false,
            }
        })
        .collect();
    let free_text = options.iter().any(is_free_text_option);
    Some(AgentQuestion {
        text: question_text(&lines[..first]),
        options,
        multi_select: false,
        free_text,
        source: "screen".into(),
    })
}

fn parse_option(line: &str) -> Option<AgentQuestionOption> {
    let mut rest = line.trim_start();
    let mut selected = false;
    for marker in ["❯", "›", "▶", ">", "→"] {
        if let Some(after) = rest.strip_prefix(marker) {
            selected = true;
            rest = after.trim_start();
            break;
        }
    }
    let (checkbox, checked, rest) = match rest {
        r if r.starts_with("[ ]") || r.starts_with("◻") || r.starts_with("☐") => (
            true,
            false,
            r.trim_start_matches("[ ]")
                .trim_start_matches(['◻', '☐'])
                .trim_start(),
        ),
        r if r.starts_with("[x]")
            || r.starts_with("[X]")
            || r.starts_with("[✔]")
            || r.starts_with("◼")
            || r.starts_with("☑") =>
        {
            let r = r
                .trim_start_matches("[x]")
                .trim_start_matches("[X]")
                .trim_start_matches("[✔]")
                .trim_start_matches(['◼', '☑']);
            (true, true, r.trim_start())
        }
        r => (false, false, r),
    };
    // `1. Yes`, `2) No`, `[a] Trust`, `(y) Yes`
    let (key, label) = if let Some((number, label)) = split_numbered(rest) {
        (Some(number), label)
    } else if let Some((key, label)) = split_bracket_key(rest) {
        (Some(key), label)
    } else if checkbox && !rest.is_empty() {
        (None, rest)
    } else {
        return None;
    };
    let label = label.trim();
    if label.is_empty() || label.len() > 160 {
        return None;
    }
    Some(AgentQuestionOption {
        label: label.to_string(),
        key,
        description: None,
        selected,
        checkbox,
        checked,
    })
}

fn split_numbered(text: &str) -> Option<(String, &str)> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 2 {
        return None;
    }
    let rest = &text[digits.len()..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    rest.starts_with(' ').then_some((digits, rest))
}

fn split_bracket_key(text: &str) -> Option<(String, &str)> {
    for (open, close) in [('[', ']'), ('(', ')')] {
        let Some(inner) = text.strip_prefix(open) else {
            continue;
        };
        let (key, rest) = inner.split_once(close)?;
        if (1..=3).contains(&key.chars().count())
            && key.chars().all(|ch| ch.is_alphanumeric())
            && rest.starts_with(' ')
        {
            return Some((key.to_string(), rest));
        }
    }
    None
}

/// An indented continuation line under an option.
fn is_description(line: &str) -> bool {
    let indent = line.len() - line.trim_start().len();
    indent >= 5 && parse_option(line).is_none() && !line.trim().is_empty()
}

fn is_rule(line: &str) -> bool {
    line.chars().count() >= 3 && line.chars().all(|ch| "─━═-_╭╮╰╯".contains(ch))
}

/// The text inside a `│ … │` box line.
fn strip_box(line: &str) -> &str {
    let trimmed = line.trim();
    match trimmed
        .strip_prefix('│')
        .and_then(|inner| inner.strip_suffix('│'))
    {
        Some(inner) => inner,
        None => line,
    }
}

fn read_agent(
    request_id: &str,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<(AgentStatus, String), QuestionError> {
    let agent = app_value(
        request_id,
        Method::AgentGet(crate::api::schema::AgentTarget {
            target: pane_id.to_string(),
        }),
        api_tx,
    )?;
    let status = serde_json::from_value(agent["result"]["agent"]["agent_status"].clone())
        .unwrap_or(AgentStatus::Unknown);
    let read = app_value(
        request_id,
        Method::PaneRead(crate::api::schema::PaneReadParams {
            pane_id: pane_id.to_string(),
            source: ReadSource::Visible,
            lines: None,
            format: ReadFormat::Text,
            strip_ansi: true,
            intent: crate::api::schema::ReadIntent::Passive,
        }),
        api_tx,
    )?;
    let text = read["result"]["read"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Ok((status, text))
}

fn app(request_id: &str, method: Method, api_tx: &ApiRequestSender) -> Result<(), QuestionError> {
    app_value(request_id, method, api_tx).map(|_| ())
}

fn app_value(
    request_id: &str,
    method: Method,
    api_tx: &ApiRequestSender,
) -> Result<serde_json::Value, QuestionError> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: format!("{request_id}:question"),
            method,
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).unwrap_or_default();
    if let Some(error) = value.get("error") {
        return Err((
            "agent_not_found",
            error["message"]
                .as_str()
                .unwrap_or("request failed")
                .to_string(),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(option: Option<&str>, options: &[&str], text: Option<&str>) -> AgentAnswerParams {
        AgentAnswerParams {
            pane_id: "w1:p1".into(),
            option: option.map(str::to_string),
            options: options.iter().map(|option| option.to_string()).collect(),
            text: text.map(str::to_string),
        }
    }

    #[test]
    fn numbered_list_with_a_selection_marker() {
        let screen = "old text\n1. not this\n\n───────\n Do you want to proceed?\n ❯ 1. Yes\n   2. No, and tell Claude what to do\n\n Esc to cancel\n";
        let question = parse_question(screen).unwrap();
        assert_eq!(question.text, "Do you want to proceed?");
        assert_eq!(question.options.len(), 2);
        assert!(question.options[0].selected);
        assert_eq!(question.options[1].key.as_deref(), Some("2"));
        assert!(question.free_text);
        assert!(!question.multi_select);
        assert_eq!(
            answer_keys(&question, &params(Some("2"), &[], None)).unwrap(),
            [AnswerInput::Keys(vec!["down".into(), "enter".into()])]
        );
    }

    #[test]
    fn keyed_options_inside_a_box_use_their_key() {
        let screen = "  ╭──────╮\n  │  Do you trust the contents of this directory?  │\n  │                │\n  │    /tmp/some/folder   │\n  │                │\n  │  ▶ [a] Trust this workspace  │\n  │    [q] Quit   │\n  ╰──────╯\n";
        let question = parse_question(screen).unwrap();
        assert_eq!(
            question.text,
            "Do you trust the contents of this directory?"
        );
        assert_eq!(question.options[0].label, "Trust this workspace");
        assert_eq!(
            answer_keys(&question, &params(Some("q"), &[], None)).unwrap(),
            [AnswerInput::Keys(vec!["q".into()])]
        );
    }

    #[test]
    fn checkbox_lists_are_multi_select() {
        let screen = "Which parts?\n❯ [ ] Backend\n  [x] Frontend\n  [ ] Docs\n";
        let question = parse_question(screen).unwrap();
        assert!(question.multi_select);
        assert!(question.options[1].checked);
        assert_eq!(
            answer_keys(&question, &params(None, &["Docs", "Frontend"], None)).unwrap(),
            [AnswerInput::Keys(vec![
                "down".into(),
                "down".into(),
                "space".into(),
                "enter".into()
            ])]
        );
        assert_eq!(
            answer_keys(&question, &params(Some("Docs"), &[], Some("x")))
                .unwrap_err()
                .0,
            "free_text_not_allowed"
        );
    }

    #[test]
    fn free_text_picks_the_type_option_first() {
        let screen = "Which color?\n❯ 1. Red\n  2. Blue\n  3. Type something.\n";
        let question = parse_question(screen).unwrap();
        assert!(question.free_text);
        assert_eq!(
            answer_keys(&question, &params(None, &[], Some("green"))).unwrap(),
            [
                AnswerInput::Keys(vec!["down".into(), "down".into(), "enter".into()]),
                AnswerInput::Text("green".into()),
                AnswerInput::Keys(vec!["enter".into()]),
            ]
        );
    }

    #[test]
    fn unkeyed_lists_use_the_cursor_marker_and_its_column() {
        let screen = " Accessing workspace:\n /tmp/folder\n Quick safety check: Is this a project you trust? (Like your own code)\n Claude can read files here.\n Security guide\n ❯ No, exit\n   Yes, I trust this folder\n\n Enter to confirm · Esc to cancel\n";
        let question = parse_question(screen).unwrap();
        assert_eq!(
            question.text,
            "Accessing workspace:\n/tmp/folder\nQuick safety check: Is this a project you trust? (Like your own code)\nClaude can read files here.\nSecurity guide"
        );
        let labels: Vec<_> = question
            .options
            .iter()
            .map(|option| option.label.as_str())
            .collect();
        assert_eq!(labels, ["No, exit", "Yes, I trust this folder"]);
        assert_eq!(
            answer_keys(
                &question,
                &params(Some("Yes, I trust this folder"), &[], None)
            )
            .unwrap(),
            [AnswerInput::Keys(vec!["down".into(), "enter".into()])]
        );
    }

    #[test]
    fn plain_text_has_no_question() {
        assert!(parse_question("just output\nmore output\n").is_none());
        assert!(parse_question("1. only one option\n").is_none());
    }
}
