//! A palette to pick a machine by typing part of its name, for a new tab to open on.

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseButton, Window, div, prelude::*, px,
};

use crate::daemon::Machine;
use crate::theme;

/// What came of the palette: a machine picked, or none.
pub enum Picked {
    Machine(Machine),
    Dismissed,
}

pub struct MachinePalette {
    machines: Vec<Machine>,
    query: String,
    /// The highlighted machine, among those that match.
    selected: usize,
    focus: FocusHandle,
}

impl EventEmitter<Picked> for MachinePalette {}

impl Focusable for MachinePalette {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MachinePalette {
    pub fn new(machines: Vec<Machine>, cx: &mut Context<Self>) -> Self {
        Self {
            machines,
            query: String::new(),
            selected: 0,
            focus: cx.focus_handle(),
        }
    }

    /// The machines whose name or alias has the query in it, ignoring case.
    fn matches(&self) -> Vec<&Machine> {
        let query = self.query.to_lowercase();
        self.machines
            .iter()
            .filter(|machine| {
                machine.label().to_lowercase().contains(&query) || machine.name.to_lowercase().contains(&query)
            })
            .collect()
    }

    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(machine) = self.matches().get(index) {
            cx.emit(Picked::Machine((*machine).clone()));
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let count = self.matches().len();
        match keystroke.key.as_str() {
            "escape" => cx.emit(Picked::Dismissed),
            "enter" => self.pick(self.selected, cx),
            "up" if count > 0 => self.selected = (self.selected + count - 1) % count,
            "down" if count > 0 => self.selected = (self.selected + 1) % count,
            "backspace" => {
                self.query.pop();
                self.selected = 0;
            }
            _ => {
                let m = &keystroke.modifiers;
                let typed = keystroke.key_char.as_deref().filter(|_| !m.control && !m.platform);
                match typed.filter(|text| !text.chars().any(char::is_control)) {
                    Some(text) => {
                        self.query.push_str(text);
                        self.selected = 0;
                    }
                    // Not for the palette: let the window's shortcuts have it.
                    None => return,
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for MachinePalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = self.matches();
        let items: Vec<_> = matches
            .iter()
            .enumerate()
            .map(|(index, machine)| {
                let selected = index == self.selected;
                div()
                    .id(("machine", index))
                    .flex()
                    .justify_between()
                    .gap_4()
                    .px_3()
                    .py_1p5()
                    .rounded_md()
                    .text_sm()
                    .text_color(theme::foreground())
                    .when(selected, |item| item.bg(theme::tab_active()))
                    .when(!selected, |item| item.hover(|item| item.bg(theme::tab_hover())))
                    .child(machine.label().to_owned())
                    .when(machine.label() != machine.name, |item| {
                        item.child(div().text_color(theme::muted()).child(machine.name.clone()))
                    })
                    .on_click(cx.listener(move |palette, _, _, cx| palette.pick(index, cx)))
            })
            .collect();
        let empty = items.is_empty();

        let caret = div().w(px(1.)).h(px(16.)).bg(theme::foreground());
        // The caret goes after what's typed, or before the hint while nothing is.
        let field = if self.query.is_empty() {
            vec![caret, div().text_color(theme::muted()).child("Open a tab on…")]
        } else {
            vec![div().text_color(theme::foreground()).child(self.query.clone()), caret]
        };
        div()
            .id("machine-palette")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .occlude()
            .w(px(420.))
            .max_w_full()
            .flex()
            .flex_col()
            .bg(theme::titlebar())
            .border_1()
            .border_color(theme::border())
            .rounded_lg()
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(Picked::Dismissed)))
            // Clicks inside stay with the palette, rather than reach the terminal under it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme::border())
                    .text_sm()
                    .children(field),
            )
            .child(
                div()
                    .id("machine-palette-list")
                    .flex()
                    .flex_col()
                    .p_1()
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .children(items)
                    .when(empty, |list| {
                        list.child(
                            div()
                                .px_3()
                                .py_1p5()
                                .text_sm()
                                .text_color(theme::muted())
                                .child("No machines match"),
                        )
                    }),
            )
    }
}
