// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! An inline-style adapter over egui's native selection and IME implementation.

use egui::{
    text::{CCursorRange, CharIndex, LayoutJob},
    *,
};
use ptouch_render::styled_text::{StylePatch, StyleSpan, StyledText, TextStyle};
use std::{
    collections::HashSet,
    ops::Range,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Snapshot {
    document: StyledText,
    selection: Option<CCursorRange>,
    pending: TextStyle,
    pending_at: Option<usize>,
}

#[derive(Clone, Default)]
struct Session {
    last_action: f64,
    current: Snapshot,
    committed: StyledText,
    undo: Vec<Arc<Snapshot>>,
    redo: Vec<Arc<Snapshot>>,
    composition: Option<Snapshot>,
    typing_group: Option<(f64, Option<CCursorRange>)>,
    font_search: String,
}

impl Session {
    fn new(document: StyledText) -> Self {
        Self {
            current: Snapshot {
                document: document.clone(),
                ..Default::default()
            },
            committed: document,
            ..Default::default()
        }
    }

    fn record(&mut self, before: Snapshot, typing: bool, time: f64) {
        if before.document == self.current.document && before.pending == self.current.pending {
            return;
        }
        self.last_action = time;
        let coalesce = typing
            && self
                .typing_group
                .is_some_and(|(last, caret)| time - last < 1.0 && caret == before.selection);
        if !coalesce {
            self.undo.push(Arc::new(before));
        }
        self.redo.clear();
        // Bounded full-state history: styles and insertion attributes participate.
        while self.undo.len() > 100
            || self
                .undo
                .iter()
                .map(|s| s.document.text.len() + s.document.spans.len() * 128)
                .sum::<usize>()
                > 8_388_608
        {
            self.undo.remove(0);
        }
        self.typing_group = typing.then_some((time, self.current.selection));
    }

    fn history(&mut self, redo: bool) -> bool {
        let (source, destination) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let Some(previous) = source.pop() else {
            return false;
        };
        destination.push(Arc::new(self.current.clone()));
        self.current = (*previous).clone();
        self.committed = self.current.document.clone();
        self.composition = None;
        self.typing_group = None;
        true
    }

    fn format(&mut self, patch: StylePatch, time: f64) {
        let before = self.current.clone();
        let range = selected_range(self.current.selection);
        if range.is_empty() {
            patch.apply(&mut self.current.pending);
            self.current.pending_at = Some(range.start);
        } else {
            self.current.document.format(range, &patch);
            self.current.pending = self
                .current
                .document
                .style_at(selected_range(self.current.selection).start);
        }
        self.record(before, false, time);
        self.committed = self.current.document.clone();
    }
}

/// Insertion formatting is transient editor state, so it does not create a
/// document snapshot. Route only such a newer action ahead of document undo.
pub fn pending_history(ctx: &Context, id: Id, redo: bool, after: f64) -> bool {
    let session_id = id.with("session");
    let Some(mut session) = ctx.data_mut(|d| d.get_temp::<Session>(session_id)) else {
        return false;
    };
    let source = if redo { &session.redo } else { &session.undo };
    if session.last_action < after
        || source
            .last()
            .is_none_or(|s| s.document != session.current.document)
    {
        return false;
    }
    if !session.history(redo) {
        return false;
    }
    session.last_action = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(session_id, session));
    ctx.request_repaint();
    true
}

pub fn is_composing(ctx: &Context, id: Id) -> bool {
    ctx.data_mut(|d| d.get_temp::<Session>(id.with("session")))
        .is_some_and(|s| s.composition.is_some())
}

#[derive(Clone)]
struct ClipboardFragment {
    document: StyledText,
}

struct StyledBuffer {
    document: StyledText,
    pending: TextStyle,
    styles: Arc<Mutex<Vec<StyleSpan>>>,
    paste: Option<StyledText>,
}

impl StyledBuffer {
    fn publish(&self) {
        *self.styles.lock().unwrap() = self.document.spans.clone();
    }
}

impl TextBuffer for StyledBuffer {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        &self.document.text
    }
    fn insert_text(&mut self, text: &str, at: CharIndex) -> usize {
        if !self.document.replace(at.0..at.0, text, &self.pending) {
            return 0;
        }
        if let Some(fragment) = self.paste.take().filter(|f| f.text == text) {
            for span in fragment.spans {
                self.document.format(
                    at.0 + span.start..at.0 + span.end,
                    &StylePatch {
                        font_name: Some(span.style.font_name),
                        font_size: Some(span.style.font_size),
                        bold: Some(span.style.bold),
                        italic: Some(span.style.italic),
                        underline: Some(span.style.underline),
                    },
                );
            }
        }
        self.publish();
        text.chars().count()
    }
    fn delete_char_range(&mut self, range: Range<CharIndex>) {
        self.document
            .replace(range.start.0..range.end.0, "", &self.pending);
        self.publish();
    }
}

fn selected_range(selection: Option<CCursorRange>) -> Range<usize> {
    selection.map_or(0..0, |s| {
        let [a, b] = s.sorted_cursors();
        a.index.0..b.index.0
    })
}

fn selection_style(session: &Session) -> TextStyle {
    let range = selected_range(session.current.selection);
    let mut style = session.current.pending.clone();
    if !range.is_empty() {
        let regions = ptouch_render::styled_text::style_regions(
            &session.current.document.text,
            &session.current.document.spans,
        );
        let selected: Vec<_> = regions
            .iter()
            .filter(|(r, _)| r.start < range.end && range.start < r.end)
            .collect();
        style.bold = selected.iter().all(|(_, s)| s.bold);
        style.italic = selected.iter().all(|(_, s)| s.italic);
        style.underline = selected.iter().all(|(_, s)| s.underline);
    }
    style
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToggleState {
    Off,
    On,
    Mixed,
}

fn selection_toggle(session: &Session, attribute: fn(&TextStyle) -> bool) -> ToggleState {
    let range = selected_range(session.current.selection);
    let value = attribute(&session.current.pending);
    if range.is_empty() {
        return if value {
            ToggleState::On
        } else {
            ToggleState::Off
        };
    }
    let regions = ptouch_render::styled_text::style_regions(
        &session.current.document.text,
        &session.current.document.spans,
    );
    let mut values = regions
        .iter()
        .filter(|(r, _)| r.start < range.end && range.start < r.end)
        .map(|(_, style)| attribute(style));
    let first = values.next().unwrap_or(value);
    if values.any(|value| value != first) {
        ToggleState::Mixed
    } else if first {
        ToggleState::On
    } else {
        ToggleState::Off
    }
}

fn format_button(ui: &mut Ui, glyph: &str, name: &str, state: ToggleState) -> Response {
    let label = if state == ToggleState::Mixed {
        format!("{glyph} —")
    } else {
        glyph.into()
    };
    let text = RichText::new(label);
    let text = match glyph {
        "B" => text.strong(),
        "I" => text.italics(),
        "U" => text.underline(),
        _ => text,
    };
    let description = match state {
        ToggleState::Mixed => format!("{name}: mixed. Apply to the selected text."),
        ToggleState::On => format!("{name}: on. Click to turn off."),
        ToggleState::Off => format!("{name}: off. Click to turn on."),
    };
    let response = ui.add(
        Button::new(text)
            .selected(state != ToggleState::Off)
            .min_size(vec2(30.0, 24.0)),
    );
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::Button,
            ui.is_enabled(),
            state == ToggleState::On,
            &description,
        )
    });
    response.on_hover_text(description)
}

fn inherited_style(document: &StyledText, at: usize) -> TextStyle {
    document.style_at(if at == 0 { 0 } else { at - 1 })
}

fn editing_format(
    ctx: &Context,
    style: &TextStyle,
    default_font: &str,
    base_size: f32,
    color: Color32,
) -> TextFormat {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    let name = style.font_name.as_deref().unwrap_or(default_font);
    let key = format!("inline:{name}:{}:{}", style.bold, style.italic);
    let family = FontFamily::Name(key.clone().into());
    let cache_id = Id::new("inline_font_faces");
    let mut loaded = ctx
        .data_mut(|data| data.get_temp::<HashSet<String>>(cache_id))
        .unwrap_or_default();
    if !loaded.contains(&key) && loaded.len() < 64 {
        loaded.insert(key.clone());
        if let Some((bytes, index)) = ptouch_render::font::face_data(name, style.bold, style.italic)
        {
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            ctx.add_font(FontInsert {
                name: key.clone(),
                data,
                families: vec![InsertFontFamily {
                    family: family.clone(),
                    priority: FontPriority::Highest,
                }],
            });
            for (suffix, bytes) in [
                (
                    "cjk",
                    include_bytes!("../assets/fonts/DroidSansFallback.ttf").as_slice(),
                ),
                (
                    "emoji",
                    include_bytes!("../assets/fonts/NotoEmoji.ttf").as_slice(),
                ),
            ] {
                ctx.add_font(FontInsert {
                    name: format!("{key}:{suffix}"),
                    data: FontData::from_static(bytes),
                    families: vec![InsertFontFamily {
                        family: family.clone(),
                        priority: FontPriority::Lowest,
                    }],
                });
            }
            ctx.request_repaint();
        }
        ctx.data_mut(|data| data.insert_temp(cache_id, loaded));
    }
    let available = ctx.fonts(|fonts| fonts.families().contains(&family));
    TextFormat {
        font_id: FontId::new(
            (style.font_size.unwrap_or(base_size) / base_size * 16.0).clamp(4.0, 128.0),
            if available {
                family
            } else {
                FontFamily::Proportional
            },
        ),
        color,
        italics: style.italic && !available,
        underline: if style.underline {
            Stroke::new(1.0, color)
        } else {
            Stroke::NONE
        },
        ..Default::default()
    }
}

fn layout_job(
    text: &str,
    styles: &[StyleSpan],
    formats: &[TextFormat],
    default: &TextFormat,
    wrap: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut bytes: Vec<usize> = text.char_indices().map(|p| p.0).collect();
    bytes.push(text.len());
    let mut cursor = 0;
    for (span, format) in styles.iter().zip(formats) {
        if cursor < span.start {
            job.append(
                &text[bytes[cursor]..bytes[span.start]],
                0.0,
                default.clone(),
            );
        }
        job.append(
            &text[bytes[span.start]..bytes[span.end]],
            0.0,
            format.clone(),
        );
        cursor = span.end;
    }
    if cursor < bytes.len() - 1 {
        job.append(&text[bytes[cursor]..], 0.0, default.clone());
    }
    job
}

enum Action {
    Format(StylePatch),
    Undo,
    Redo,
}

/// Changes the authoritative text only after IME composition is committed.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    content: &mut String,
    spans: &mut Vec<StyleSpan>,
    id: Id,
    default_font: &str,
    base_size: f32,
    fonts: &[String],
) -> bool {
    let base_size = if base_size.is_finite() && base_size > 0.0 {
        base_size
    } else {
        12.0
    };
    let session_id = id.with("session");
    let original = StyledText {
        text: content.clone(),
        spans: spans.clone(),
    };
    let mut session = ui
        .ctx()
        .data_mut(|data| data.get_temp::<Session>(session_id))
        .unwrap_or_else(|| Session::new(original.clone()));
    if original != session.committed {
        session = Session::new(original.clone());
    }
    let mut widget_state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    let focused = ui.memory(|m| m.has_focus(id));
    // Native TextEdit may collapse its selection after losing focus. Keep the
    // formatting target while a font or size control owns keyboard focus.
    let old_selection = if focused {
        widget_state
            .cursor
            .char_range()
            .or(session.current.selection)
    } else {
        session
            .current
            .selection
            .or(widget_state.cursor.char_range())
    };
    session.current.selection = old_selection;
    let range = selected_range(old_selection);
    if session.current.pending_at != Some(range.start) || !range.is_empty() {
        session.current.pending = if range.is_empty() {
            inherited_style(&session.current.document, range.start)
        } else {
            session.current.document.style_at(range.start)
        };
        session.current.pending_at = None;
    }
    let before = session.current.clone();
    let events = ui.input(|i| i.events.clone());
    let document_history = ui.ctx().data_mut(|d| {
        d.get_temp::<bool>(Id::new("document_history"))
            .unwrap_or(false)
    });
    let mut action = None;
    if focused || !ui.ctx().text_edit_focused() {
        // More specific modifiers first: egui's logical shortcuts can accept Shift.
        for (shortcut, command) in [
            (
                KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                Action::Redo,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::Z),
                Action::Undo,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::Y),
                Action::Redo,
            ),
        ] {
            if !document_history && ui.input_mut(|i| i.consume_shortcut(&shortcut)) {
                action = Some(command);
                break;
            }
        }
        for (key, patch) in [
            (
                Key::B,
                StylePatch {
                    bold: Some(!selection_style(&session).bold),
                    ..Default::default()
                },
            ),
            (
                Key::I,
                StylePatch {
                    italic: Some(!selection_style(&session).italic),
                    ..Default::default()
                },
            ),
            (
                Key::U,
                StylePatch {
                    underline: Some(!selection_style(&session).underline),
                    ..Default::default()
                },
            ),
        ] {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) {
                action = Some(Action::Format(patch));
            }
        }
    }
    let clipboard_id = Id::new("inline_clipboard");
    if events.iter().any(|e| matches!(e, Event::Copy | Event::Cut)) {
        if focused && !range.is_empty() && session.composition.is_none() {
            let mut fragment = session.current.document.fragment(range.clone());
            // Resolve inherited family/size so copying between text boxes retains formatting.
            fragment.spans =
                ptouch_render::styled_text::style_regions(&fragment.text, &fragment.spans)
                    .into_iter()
                    .map(|(range, mut style)| {
                        style.font_name =
                            Some(style.font_name.unwrap_or_else(|| default_font.into()));
                        style.font_size = Some(style.font_size.unwrap_or(base_size));
                        StyleSpan {
                            start: range.start,
                            end: range.end,
                            style,
                        }
                    })
                    .collect();
            ui.ctx().data_mut(|d| {
                d.insert_temp(clipboard_id, ClipboardFragment { document: fragment })
            });
        } else {
            ui.ctx()
                .data_mut(|d| d.remove::<ClipboardFragment>(clipboard_id));
        }
    }
    let paste = events
        .iter()
        .find_map(|e| {
            if let Event::Paste(text) = e {
                Some(text)
            } else {
                None
            }
        })
        .and_then(|text| {
            ui.ctx()
                .data_mut(|d| d.get_temp::<ClipboardFragment>(clipboard_id))
                .filter(|f| &f.document.text == text)
        })
        .map(|f| f.document);
    if focused {
        for event in &events {
            match event {
                Event::Ime(ImeEvent::Preedit { text, .. })
                    if !text.is_empty() && session.composition.is_none() =>
                {
                    session.composition = Some(before.clone());
                }
                _ => (),
            }
        }
    }
    let shared_styles = Arc::new(Mutex::new(session.current.document.spans.clone()));
    let mut buffer = StyledBuffer {
        document: session.current.document.clone(),
        pending: session.current.pending.clone(),
        styles: shared_styles.clone(),
        paste,
    };
    let ctx = ui.ctx().clone();
    let color = ui.visuals().text_color();
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap: f32| {
        let styles = shared_styles.lock().unwrap();
        let default = editing_format(&ctx, &TextStyle::default(), default_font, base_size, color);
        let formats: Vec<_> = styles
            .iter()
            .map(|span| editing_format(&ctx, &span.style, default_font, base_size, color))
            .collect();
        let job = layout_job(text.as_str(), &styles, &formats, &default, wrap);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    // String-only undo must never restore text against unrelated style ranges.
    widget_state.clear_undoer();
    widget_state.store(ui.ctx(), id);
    let mut output = TextEdit::multiline(&mut buffer)
        .id(id)
        .desired_width(f32::INFINITY)
        .desired_rows(3)
        .layouter(&mut layouter)
        .show(ui);
    (*output.response).clone().on_hover_text(
        "Editing text is scaled for readability. The canvas shows the final print size.",
    );
    session.current.document = buffer.document;
    session.current.selection = if output.response.has_focus() {
        output.cursor_range.or(old_selection)
    } else {
        old_selection
    };
    let native_changed = output.response.changed();
    let ime_finished = focused
        && events.iter().any(|event| match event {
            Event::Ime(ImeEvent::Commit(_)) => true,
            Event::Ime(ImeEvent::Preedit { text, .. }) => text.is_empty(),
            _ => false,
        });
    if session.composition.is_some() && output.response.lost_focus() {
        session.current = session.composition.take().unwrap();
        output
            .state
            .cursor
            .set_char_range(session.current.selection);
        output.state.clear_undoer();
        output.state.clone().store(ui.ctx(), id);
    } else if session.composition.is_none() || ime_finished {
        let prior = session.composition.take().unwrap_or(before);
        let typing = events
            .iter()
            .any(|e| matches!(e, Event::Text(_) | Event::Ime(ImeEvent::Commit(_))))
            && !events.iter().any(|e| matches!(e, Event::Paste(_)));
        session.record(prior, typing, ui.input(|i| i.time));
        session.committed = session.current.document.clone();
    }
    if session.current.selection != old_selection {
        session.current.pending_at = None;
        if !native_changed {
            session.typing_group = None;
        }
        let range = selected_range(session.current.selection);
        if native_changed && session.current.document.is_empty() {
            // Deleting the last styled character keeps that insertion style.
            session.current.pending_at = Some(0);
        } else {
            session.current.pending = if range.is_empty() {
                inherited_style(&session.current.document, range.start)
            } else {
                session.current.document.style_at(range.start)
            };
        }
    }
    let selected = selected_range(session.current.selection);
    let selected_style = selection_style(&session);
    ui.label(RichText::new("Text format").strong());
    if selected.is_empty() {
        ui.small("Applies to the next text you type.");
    } else {
        ui.small(format!(
            "Applies to {} selected characters.",
            selected.len()
        ));
    }
    ui.horizontal(|ui| {
        for (label, name, state, patch) in [
            (
                "B",
                "Bold",
                selection_toggle(&session, |s| s.bold),
                StylePatch {
                    bold: Some(!selected_style.bold),
                    ..Default::default()
                },
            ),
            (
                "I",
                "Italic",
                selection_toggle(&session, |s| s.italic),
                StylePatch {
                    italic: Some(!selected_style.italic),
                    ..Default::default()
                },
            ),
            (
                "U",
                "Underline",
                selection_toggle(&session, |s| s.underline),
                StylePatch {
                    underline: Some(!selected_style.underline),
                    ..Default::default()
                },
            ),
        ] {
            if format_button(ui, label, name, state).clicked() {
                action = Some(Action::Format(patch));
            }
        }
        if !document_history
            && ui
                .add_enabled(!session.undo.is_empty(), Button::new("Undo"))
                .clicked()
        {
            action = Some(Action::Undo);
        }
        if !document_history
            && ui
                .add_enabled(!session.redo.is_empty(), Button::new("Redo"))
                .clicked()
        {
            action = Some(Action::Redo);
        }
    });
    if [
        selection_toggle(&session, |s| s.bold),
        selection_toggle(&session, |s| s.italic),
        selection_toggle(&session, |s| s.underline),
    ]
    .contains(&ToggleState::Mixed)
    {
        ui.small("— Mixed formatting in selection.");
    }
    let regions = ptouch_render::styled_text::style_regions(
        &session.current.document.text,
        &session.current.document.spans,
    );
    let selected_regions: Vec<_> = regions
        .iter()
        .filter(|(r, _)| r.start < selected.end && selected.start < r.end)
        .collect();
    let mixed_size = selected_regions.iter().any(|(_, style)| {
        style.font_size.unwrap_or(base_size) != selected_style.font_size.unwrap_or(base_size)
    });
    ui.horizontal(|ui| {
        ui.label("Size:");
        let mut size = if mixed_size {
            0.0
        } else {
            selected_style.font_size.unwrap_or(base_size)
        };
        let previous_size = size;
        ui.add(
            crate::widgets::decimal_drag(&mut size)
                .custom_formatter(|value, _| {
                    if value == 0.0 {
                        "Mixed".into()
                    } else {
                        format!("{value:.1}")
                    }
                })
                .range(0.1..=512.0)
                .speed(0.5)
                .suffix(" pt"),
        );
        // DragValue's response also changes while its draft text is edited.
        // Only the committed number may format text or return editor focus.
        if size != previous_size {
            action = Some(Action::Format(StylePatch {
                font_size: Some(Some(size)),
                ..Default::default()
            }));
        }
        if ui.button("Clear style").clicked() {
            action = Some(Action::Format(StylePatch {
                font_name: Some(None),
                font_size: Some(None),
                bold: Some(false),
                italic: Some(false),
                underline: Some(false),
            }));
        }
    });
    let mixed_font = selected_regions.iter().any(|(_, style)| {
        style.font_name.as_deref().unwrap_or(default_font)
            != selected_style.font_name.as_deref().unwrap_or(default_font)
    });
    ui.small(if selected.is_empty() {
        "Typing font:"
    } else {
        "Selection font:"
    });
    ComboBox::from_id_salt(id.with("font"))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .selected_text(if mixed_font {
            "Mixed fonts"
        } else {
            selected_style.font_name.as_deref().unwrap_or(default_font)
        })
        .width(ui.available_width().max(80.0))
        .show_ui(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut session.font_search)
                    .hint_text("Search selection font..."),
            );
            if ui
                .selectable_label(selected_style.font_name.is_none(), "Inherit label font")
                .clicked()
            {
                ui.close();
                action = Some(Action::Format(StylePatch {
                    font_name: Some(None),
                    ..Default::default()
                }));
            }
            for font in fonts.iter().filter(|name| {
                name.to_lowercase()
                    .contains(&session.font_search.to_lowercase())
            }) {
                if ui
                    .selectable_label(selected_style.font_name.as_ref() == Some(font), font)
                    .clicked()
                {
                    ui.close();
                    action = Some(Action::Format(StylePatch {
                        font_name: Some(Some(font.clone())),
                        ..Default::default()
                    }));
                }
            }
        });
    if let Some(action) = action {
        // A style click ends unfinished composition before changing the selection.
        if let Some(prior) = session.composition.take() {
            session.current = prior;
        }
        match action {
            Action::Format(patch) => session.format(patch, ui.input(|i| i.time)),
            Action::Undo => {
                session.history(false);
            }
            Action::Redo => {
                session.history(true);
            }
        }
        output
            .state
            .cursor
            .set_char_range(session.current.selection);
        output.state.clear_undoer();
        output.state.store(ui.ctx(), id);
        output.response.request_focus();
        ui.ctx().request_repaint();
    }
    let changed = original != session.committed;
    if changed {
        *content = session.committed.text.clone();
        *spans = session.committed.spans.clone();
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(session_id, session));
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::CCursor;

    struct EditorHarness {
        ctx: Context,
        id: Id,
        document: StyledText,
    }

    impl EditorHarness {
        fn new(text: &str) -> Self {
            let mut h = Self {
                ctx: Context::default(),
                id: Id::new("editor-under-test"),
                document: StyledText {
                    text: text.into(),
                    spans: vec![],
                },
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h.select(0..0);
            h
        }
        fn frame(&mut self, events: Vec<Event>) -> Vec<egui::epaint::ClippedShape> {
            let Self { ctx, id, document } = self;
            let mut output = ctx.run_ui(
                RawInput {
                    events,
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 680.0))),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui,
                        &mut document.text,
                        &mut document.spans,
                        *id,
                        "DejaVu Sans",
                        12.0,
                        &["DejaVu Serif".into()],
                    );
                },
            );
            output.textures_delta.clear();
            output.shapes
        }
        fn select(&mut self, range: Range<usize>) {
            self.ctx.memory_mut(|m| m.request_focus(self.id));
            let mut state = egui::text_edit::TextEditState::load(&self.ctx, self.id).unwrap();
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(range.start),
                CCursor::new(range.end),
            )));
            state.store(&self.ctx, self.id);
            self.frame(vec![]);
        }
        fn command(&mut self, key: Key, shift: bool) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: if shift {
                    Modifiers::COMMAND | Modifiers::SHIFT
                } else {
                    Modifiers::COMMAND
                },
            }]);
        }
    }

    #[test]
    fn formatting_and_redo_restore_styles_and_selection_without_changing_text() {
        let mut h = EditorHarness::new("端口🙂AB");
        h.select(1..3);
        h.command(Key::B, false);
        assert_eq!(h.document.text, "端口🙂AB");
        assert!(h.document.style_at(1).bold && h.document.style_at(2).bold);
        assert!(!h.document.style_at(3).bold);
        h.command(Key::Z, false);
        assert!(h.document.spans.is_empty());
        h.command(Key::Z, true);
        assert!(h.document.style_at(2).bold);
        let state = egui::text_edit::TextEditState::load(&h.ctx, h.id).unwrap();
        assert_eq!(selected_range(state.cursor.char_range()), 1..3);
    }

    #[test]
    fn size_draft_keeps_selection_and_commits_only_on_enter() {
        let mut h = EditorHarness::new("ABCD");
        h.select(0..2);
        let shapes = h.frame(vec![]);
        let pos = shapes
            .iter()
            .find_map(|clipped| {
                if let egui::epaint::Shape::Text(text) = &clipped.shape
                    && text.galley.job.text == "12.0"
                {
                    Some(text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .expect("inline size widget");
        h.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ]);
        h.frame(vec![]);
        h.frame(vec![Event::Paste("18".into())]);
        assert_eq!(h.document.text, "ABCD");
        assert!(
            h.document.spans.is_empty(),
            "numeric draft must not steal text focus"
        );
        h.frame(vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![]);
        assert_eq!(h.document.text, "ABCD");
        assert_eq!(h.document.style_at(0).font_size, Some(18.0));
        assert_eq!(h.document.style_at(1).font_size, Some(18.0));
        assert_eq!(h.document.style_at(2).font_size, None);
        h.command(Key::Z, false);
        assert!(h.document.spans.is_empty());
    }

    #[test]
    fn font_search_stays_open_and_formats_the_original_selection() {
        let mut h = EditorHarness::new("ABCD");
        h.select(0..2);
        let center = |shapes: Vec<egui::epaint::ClippedShape>, label: &str| {
            shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) if t.galley.job.text == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label}"))
        };
        let click = |pos| {
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ]
        };
        let pos = center(h.frame(vec![]), "DejaVu Sans");
        h.frame(click(pos));
        let pos = center(h.frame(vec![]), "Search selection font...");
        h.frame(click(pos));
        h.frame(vec![]);
        h.frame(vec![Event::Text("Serif".into())]);
        let pos = center(h.frame(vec![]), "DejaVu Serif");
        h.frame(click(pos));
        h.frame(vec![]);
        assert_eq!(h.document.text, "ABCD");
        assert_eq!(
            h.document.style_at(0).font_name.as_deref(),
            Some("DejaVu Serif")
        );
        assert_eq!(
            h.document.style_at(1).font_name.as_deref(),
            Some("DejaVu Serif")
        );
        assert_eq!(h.document.style_at(2).font_name, None);
        h.command(Key::Z, false);
        assert!(h.document.spans.is_empty());
    }

    #[test]
    fn mixed_bold_selection_becomes_uniform_and_is_reversible() {
        let mut h = EditorHarness::new("ABCD");
        h.select(0..2);
        h.command(Key::B, false);
        h.select(0..4);
        let original = h.document.clone();
        let shapes = h.frame(vec![]);
        assert!(shapes.iter().any(|s| matches!(&s.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == "B —")));
        assert_eq!(
            h.document, original,
            "displaying mixed formatting must not edit it"
        );
        h.command(Key::B, false);
        assert!((0..4).all(|i| h.document.style_at(i).bold));
        let shapes = h.frame(vec![]);
        assert!(!shapes.iter().any(|s| matches!(&s.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == "B —")));
        h.command(Key::Z, false);
        assert_eq!(h.document, original);
    }

    #[test]
    fn scope_hint_distinguishes_range_from_insertion_without_editing() {
        let mut h = EditorHarness::new("端口AB");
        let original = h.document.clone();
        h.select(0..2);
        let shapes = h.frame(vec![]);
        assert!(shapes.iter().any(|s| matches!(&s.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == "Applies to 2 selected characters.")));
        h.select(2..2);
        let shapes = h.frame(vec![]);
        assert!(shapes.iter().any(|s| matches!(&s.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == "Applies to the next text you type.")));
        assert_eq!(h.document, original);
    }

    #[test]
    fn pending_style_and_a_run_of_unicode_typing_have_full_state_undo() {
        let mut h = EditorHarness::new("");
        h.command(Key::B, false);
        h.frame(vec![Event::Text("端".into())]);
        h.frame(vec![Event::Text("口🙂".into())]);
        assert_eq!(h.document.text, "端口🙂");
        assert!((0..3).all(|i| h.document.style_at(i).bold));
        h.command(Key::Z, false);
        assert_eq!(h.document.text, "");
        h.command(Key::Z, false);
        let session = h
            .ctx
            .data_mut(|d| d.get_temp::<Session>(h.id.with("session")))
            .unwrap();
        assert!(!session.current.pending.bold);
        h.command(Key::Z, true);
        h.command(Key::Z, true);
        assert_eq!(h.document.text, "端口🙂");
        assert!(h.document.style_at(2).bold);
    }

    #[test]
    fn ime_drafts_are_transient_and_commit_is_one_reversible_edit() {
        let mut h = EditorHarness::new("端口");
        h.select(2..2);
        h.frame(vec![Event::Ime(ImeEvent::Preedit {
            text: "ni".into(),
            active_range_chars: None,
        })]);
        assert_eq!(h.document.text, "端口");
        h.frame(vec![Event::Ime(ImeEvent::Preedit {
            text: "你".into(),
            active_range_chars: None,
        })]);
        assert_eq!(h.document.text, "端口");
        h.frame(vec![Event::Ime(ImeEvent::Commit("你".into()))]);
        assert_eq!(h.document.text, "端口你");
        h.command(Key::Z, false);
        assert_eq!(h.document.text, "端口");
        h.command(Key::Z, true);
        assert_eq!(h.document.text, "端口你");
        h.frame(vec![Event::Ime(ImeEvent::DeleteSurrounding {
            before_chars: 1,
            after_chars: 0,
        })]);
        assert_eq!(h.document.text, "端口");
    }

    #[test]
    fn losing_focus_during_composition_discards_provisional_text() {
        let mut h = EditorHarness::new("PORT");
        h.select(4..4);
        h.frame(vec![Event::Ime(ImeEvent::Preedit {
            text: "pinyin".into(),
            active_range_chars: None,
        })]);
        h.ctx.memory_mut(|m| m.surrender_focus(h.id));
        h.frame(vec![]);
        assert_eq!(h.document.text, "PORT");
        let session = h
            .ctx
            .data_mut(|d| d.get_temp::<Session>(h.id.with("session")))
            .unwrap();
        assert!(session.composition.is_none());
        assert_eq!(session.current.document.text, "PORT");
    }

    #[test]
    fn internal_paste_preserves_sizes_styles_and_supports_undo() {
        let mut source = EditorHarness::new("A端口B");
        source.document.format(
            1..3,
            &StylePatch {
                font_size: Some(Some(18.0)),
                underline: Some(true),
                ..Default::default()
            },
        );
        source.frame(vec![]);
        source.select(1..3);
        source.frame(vec![Event::Copy]);
        let mut target = EditorHarness {
            ctx: source.ctx.clone(),
            id: Id::new("other-editor"),
            document: StyledText::default(),
        };
        target.frame(vec![]);
        target.select(0..0);
        target.frame(vec![Event::Paste("端口".into())]);
        assert_eq!(target.document.text, "端口");
        assert_eq!(target.document.style_at(1).font_size, Some(18.0));
        assert!(target.document.style_at(0).underline);
        target.command(Key::Z, false);
        assert!(target.document.text.is_empty() && target.document.spans.is_empty());
    }
    #[test]
    fn deleting_all_styled_text_retains_the_next_insertion_style() {
        let mut h = EditorHarness::new("AB");
        h.select(0..2);
        h.command(Key::B, false);
        h.frame(vec![Event::Key {
            key: Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(h.document.text.is_empty());
        h.frame(vec![Event::Text("新".into())]);
        assert!(h.document.style_at(0).bold);
    }
}
