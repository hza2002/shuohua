use crate::config::field_view::ControlKind;

/// 编辑器提交目标：写盘（现有文件）或写回内存 draft（新建流程）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditTarget {
    File(std::path::PathBuf),
    /// 新建 draft 的行 key（如 "format" / "prompt"）。
    Draft(String),
    /// Profile composer 的选中行：提交经 `ProfileComposer::commit_edit`
    /// （按已解析的 provider/component schema 校验后写 profile 文件）。
    Composer,
}

impl EditTarget {
    pub fn file_path(&self) -> Option<&std::path::PathBuf> {
        match self {
            EditTarget::File(path) => Some(path),
            EditTarget::Draft(_) | EditTarget::Composer => None,
        }
    }

    pub fn draft_key(&self) -> Option<&str> {
        match self {
            EditTarget::Draft(key) => Some(key),
            EditTarget::File(_) | EditTarget::Composer => None,
        }
    }

    pub fn is_composer(&self) -> bool {
        matches!(self, EditTarget::Composer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Multiline,
    Array,
    Secret,
    KeyCapture,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModalEditor {
    pub field_path: String,
    pub target: EditTarget,
    pub kind: ModalKind,
    pub buffer: String,
    pub cursor: usize,
}

impl ModalEditor {
    pub fn kind_for(control: &ControlKind, secret: bool) -> Option<ModalKind> {
        match control {
            ControlKind::MultilineText => Some(ModalKind::Multiline),
            ControlKind::Array => Some(ModalKind::Array),
            ControlKind::KeyCapture => Some(ModalKind::KeyCapture),
            _ if secret => Some(ModalKind::Secret),
            _ => None,
        }
    }

    pub fn new(field_path: String, target: EditTarget, kind: ModalKind, initial: String) -> Self {
        // secret starts empty (never prefill the on-disk secret); others prefill full value.
        let buffer = if kind == ModalKind::Secret {
            String::new()
        } else {
            initial
        };
        Self {
            field_path,
            target,
            kind,
            cursor: buffer.len(),
            buffer,
        }
    }

    pub fn push_char(&mut self, ch: char) {
        self.insert_str(&ch.to_string());
    }

    pub fn insert_str(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub fn newline(&mut self) {
        if matches!(self.kind, ModalKind::Multiline | ModalKind::Array) {
            self.insert_str("\n");
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        self.buffer.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(idx, _)| idx)
            .unwrap_or(0);
    }

    pub fn move_right(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        self.cursor += self.buffer[self.cursor..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(0);
    }

    /// Byte range `[start, end)` of the line the cursor is on (`end` excludes
    /// the trailing newline).
    fn line_bounds(&self) -> (usize, usize) {
        let start = self.buffer[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = self.buffer[self.cursor..]
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.buffer.len());
        (start, end)
    }

    /// Byte index of the `col`-th char within `[line_start, line_end)`, clamped
    /// to `line_end` when the line is shorter than `col`.
    fn byte_for_col(&self, line_start: usize, line_end: usize, col: usize) -> usize {
        match self.buffer[line_start..line_end].char_indices().nth(col) {
            Some((off, _)) => line_start + off,
            None => line_end,
        }
    }

    pub fn move_up(&mut self) {
        let (start, _) = self.line_bounds();
        if start == 0 {
            return; // already on the first line
        }
        let col = self.buffer[start..self.cursor].chars().count();
        let prev_nl = start - 1;
        let prev_start = self.buffer[..prev_nl]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        self.cursor = self.byte_for_col(prev_start, prev_nl, col);
    }

    pub fn move_down(&mut self) {
        let (start, end) = self.line_bounds();
        if end >= self.buffer.len() {
            return; // already on the last line
        }
        let col = self.buffer[start..self.cursor].chars().count();
        let next_start = end + 1; // skip the newline
        let next_end = self.buffer[next_start..]
            .find('\n')
            .map(|i| next_start + i)
            .unwrap_or(self.buffer.len());
        self.cursor = self.byte_for_col(next_start, next_end, col);
    }

    pub fn array_items(&self) -> Vec<String> {
        self.buffer
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// secret blank means cancel (do not overwrite). None = do not write.
    pub fn value_to_save(&self) -> Option<&str> {
        if self.kind == ModalKind::Secret && self.buffer.is_empty() {
            return None;
        }
        Some(&self.buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::field_view::ControlKind;

    #[test]
    fn secret_starts_empty_and_blank_save_is_cancel() {
        let m = ModalEditor::new(
            "api_key".into(),
            EditTarget::File("/x".into()),
            ModalKind::Secret,
            "should-ignore".into(),
        );
        assert_eq!(m.buffer, "");
        assert_eq!(m.value_to_save(), None);
    }

    #[test]
    fn multiline_prefills_and_newline_appends() {
        let mut m = ModalEditor::new(
            "prompt".into(),
            EditTarget::File("/x".into()),
            ModalKind::Multiline,
            "a".into(),
        );
        assert_eq!(m.buffer, "a");
        m.newline();
        m.push_char('b');
        assert_eq!(m.buffer, "a\nb");
        assert_eq!(m.value_to_save(), Some("a\nb"));
    }

    #[test]
    fn cursor_inserts_and_moves_on_char_boundaries() {
        let mut m = ModalEditor::new(
            "prompt".into(),
            EditTarget::File("/x".into()),
            ModalKind::Multiline,
            "你b".into(),
        );

        m.move_left();
        m.insert_str("好");

        assert_eq!(m.buffer, "你好b");
        assert_eq!(m.cursor, "你好".len());
        m.move_right();
        assert_eq!(m.cursor, m.buffer.len());
    }

    #[test]
    fn vertical_movement_keeps_column_across_lines() {
        // Lines: "abc" | "defg" | "hi"; start at end (line "hi", col 2).
        let mut m = ModalEditor::new(
            "prompt".into(),
            EditTarget::File("/x".into()),
            ModalKind::Multiline,
            "abc\ndefg\nhi".into(),
        );
        assert_eq!(m.cursor, m.buffer.len());

        // Up -> "defg" at column 2 (the 'f').
        m.move_up();
        assert_eq!(m.cursor, 6);
        // Up again -> "abc" at column 2 (the 'c').
        m.move_up();
        assert_eq!(m.cursor, 2);
        // Up at the first line is a no-op.
        m.move_up();
        assert_eq!(m.cursor, 2);

        // Down -> back to "defg" column 2.
        m.move_down();
        assert_eq!(m.cursor, 6);
    }

    #[test]
    fn vertical_movement_clamps_to_shorter_line_end() {
        // Cursor on the long middle line past the short next line's length.
        let mut m = ModalEditor::new(
            "prompt".into(),
            EditTarget::File("/x".into()),
            ModalKind::Multiline,
            "hello\nhi\nworld".into(),
        );
        // Put the cursor at the end of "hello" (col 5).
        m.cursor = 5;
        // Down -> "hi" is only 2 long, so clamp to its end (byte 8).
        m.move_down();
        assert_eq!(m.cursor, 8);
    }

    #[test]
    fn array_modal_splits_non_empty_trimmed_lines() {
        let m = ModalEditor::new(
            "asr.hotwords".into(),
            EditTarget::File("/x".into()),
            ModalKind::Array,
            " Rust \n\n tokio ".into(),
        );

        assert_eq!(
            m.array_items(),
            vec!["Rust".to_string(), "tokio".to_string()]
        );
    }

    #[test]
    fn kind_for_routes_control_and_secret() {
        assert_eq!(
            ModalEditor::kind_for(&ControlKind::MultilineText, false),
            Some(ModalKind::Multiline)
        );
        assert_eq!(
            ModalEditor::kind_for(&ControlKind::KeyCapture, false),
            Some(ModalKind::KeyCapture)
        );
        assert_eq!(
            ModalEditor::kind_for(&ControlKind::Array, false),
            Some(ModalKind::Array)
        );
        assert_eq!(
            ModalEditor::kind_for(&ControlKind::Text, true),
            Some(ModalKind::Secret)
        );
        assert_eq!(ModalEditor::kind_for(&ControlKind::Text, false), None);
    }
}
