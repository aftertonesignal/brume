// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Style closures + chrome helpers shared across pages.
//!
//! Pages reach for these via `crate::pick_list_style`, `crate::panel`,
//! etc. — the names are re-exported at the crate root via `pub use`
//! in `lib.rs` so the page modules can keep their `use crate::{...}`
//! lists tidy.
//!
//! The visual tokens themselves (PANEL_BG, RULE_LINE, etc.) still
//! live in `lib.rs` for now — they're Brume's brand palette, not
//! widget mechanics, and several pages reference them directly.

use iced::widget::{button, column, container, horizontal_rule, horizontal_space, row, text};
use iced::{Alignment, Element, Length, Theme};

use crate::Message;
use crate::{LABEL_MUTED, PANEL_BG, PANEL_BORDER, RULE_LINE, TEXT_BRIGHT, TEXT_DIM, TITLE_BAR_BG};

/// Style closure for every `pick_list` in the UI. Iced's default
/// pick_list paints a light-grey field that reads as a foreign UI
/// chip in the warm-dark Brume palette; we override to PANEL_BG so
/// the field melts into whatever panel hosts it. Border + handle
/// stay subtle so the affordance still reads as tappable.
pub(crate) fn pick_list_style(
    _theme: &Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    let active = iced::widget::pick_list::Style {
        text_color: TEXT_BRIGHT,
        placeholder_color: LABEL_MUTED,
        handle_color: TEXT_DIM,
        background: PANEL_BG.into(),
        border: iced::Border {
            color: iced::Color {
                a: 0.18,
                ..iced::Color::WHITE
            },
            width: 1.0,
            radius: 3.0.into(),
        },
    };
    match status {
        iced::widget::pick_list::Status::Active => active,
        iced::widget::pick_list::Status::Hovered | iced::widget::pick_list::Status::Opened => {
            iced::widget::pick_list::Style {
                border: iced::Border {
                    color: iced::Color {
                        a: 0.36,
                        ..iced::Color::WHITE
                    },
                    ..active.border
                },
                ..active
            }
        }
    }
}

/// Companion style for the dropdown popover that the pick_list opens.
/// Same warm-dark fill as the field so the popover reads as an
/// extension of the panel rather than a system widget. Selected
/// option lifts on a slightly lighter wash.
pub(crate) fn pick_list_menu_style(_theme: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: PANEL_BG.into(),
        border: iced::Border {
            color: iced::Color {
                a: 0.22,
                ..iced::Color::WHITE
            },
            width: 1.0,
            radius: 2.0.into(),
        },
        text_color: TEXT_BRIGHT,
        selected_text_color: TEXT_BRIGHT,
        selected_background: iced::Color {
            r: 0.12,
            g: 0.10,
            b: 0.07,
            a: 1.0,
        }
        .into(),
    }
}

// ── Menu bar button styling ───────────────────────────────────────
//
// Active state: a low-alpha colored pill behind the label. The
// label's font size, weight, and position stay exactly the same as
// inactive state, so switching tabs never produces a layout shift.

pub(crate) fn menu_btn_style(active: bool, pill_color: iced::Color) -> button::Style {
    pill_btn_style(active, pill_color, 0.10)
}

pub(crate) fn tab_btn_style(active: bool, pill_color: iced::Color) -> button::Style {
    pill_btn_style(active, pill_color, 0.07)
}

pub(crate) fn pill_btn_style(active: bool, pill_color: iced::Color, alpha: f32) -> button::Style {
    let bg = if active {
        Some(
            iced::Color {
                a: alpha,
                ..pill_color
            }
            .into(),
        )
    } else {
        None
    };
    button::Style {
        background: bg,
        text_color: iced::Color::WHITE, // overridden by the label's own color
        border: iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: 4.0.into(),
        },
        shadow: iced::Shadow::default(),
    }
}

// ── Phosphor glow ─────────────────────────────────────────────────
//
// Brume's content uses neon phosphor accents on near-black grounds.
// iced doesn't ship a text-shadow primitive, but `container::Style`
// supports `Shadow { color, offset, blur_radius }` which radiates
// from the container's bounding box. Wrapping a small filled-circle
// element (the LFO/SEQ source dots) gives a circular phosphor halo
// that follows the glyph since the glyph is round and tiny.
//
// Don't use this on text glyphs — the shadow paints around the
// bounding rect, not the glyph shape, and reads as a rectangular
// frame around the word. True text-shape-following glow needs a
// custom widget that does a two-pass `fill_text` (Phase 5 polish).

#[allow(dead_code)]
pub(crate) fn phosphor_glow<'a>(
    inner: Element<'a, Message>,
    color: iced::Color,
    blur: f32,
) -> Element<'a, Message> {
    container(inner)
        .padding(0)
        .style(move |_t: &Theme| container::Style {
            background: None,
            border: iced::Border::default(),
            shadow: iced::Shadow {
                color,
                offset: iced::Vector::new(0.0, 0.0),
                blur_radius: blur,
            },
            ..Default::default()
        })
        .into()
}

// ── Panel helpers ─────────────────────────────────────────────────
//
// The "panel" is the Brume UI's primary chrome unit: a rounded-rect
// region with a flat dark warm-black ground, a 1 px hairline border,
// a small-caps label header, and a hairline rule under the header.

pub(crate) fn panel<'a>(label: &'static str, body: Element<'a, Message>) -> Element<'a, Message> {
    panel_inner(label, None, body)
}

/// Panel variant that hosts a single right-aligned action button in
/// its header (e.g. PANIC on CONTROLS, SAVE on LIBRARY). Tap on the
/// pill emits the supplied `on_press` Message.
pub(crate) fn panel_with_action<'a>(
    label: &'static str,
    action_label: &'static str,
    action_color: iced::Color,
    on_press: Message,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    panel_inner(label, Some((action_label, action_color, on_press)), body)
}

fn panel_inner<'a>(
    label: &'static str,
    action: Option<(&'static str, iced::Color, Message)>,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let mut header = row![
        text(label).size(10).color(LABEL_MUTED),
        horizontal_space().width(Length::Fill),
    ]
    .align_y(Alignment::Center);

    if let Some((action_label, action_color, on_press)) = action {
        let pill = button(text(action_label).size(9).color(action_color))
            .padding([2, 8])
            .on_press(on_press)
            .style(move |_t, _s| button::Style {
                background: Some(iced::Color::from_rgb(0.10, 0.08, 0.06).into()),
                text_color: action_color,
                border: iced::Border {
                    color: iced::Color {
                        a: 0.33,
                        ..action_color
                    },
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            });
        header = header.push(pill);
    }

    let chrome = column![
        container(header)
            .padding([6, 12])
            .width(Length::Fill)
            .style(|_t: &Theme| container::Style {
                background: Some(TITLE_BAR_BG.into()),
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    // Top-only radius so the title bar fill follows
                    // the panel's rounded outer corners. Bottom edges
                    // stay square so the hairline rule below sits
                    // flush against the bar.
                    radius: iced::border::top(2.0),
                },
                ..Default::default()
            }),
        horizontal_rule(1).style(|_t| iced::widget::rule::Style {
            color: RULE_LINE,
            width: 1,
            radius: 0.0.into(),
            fill_mode: iced::widget::rule::FillMode::Full,
        }),
        body,
    ]
    .spacing(0);

    container(chrome)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &Theme| container::Style {
            background: Some(PANEL_BG.into()),
            border: iced::Border {
                color: PANEL_BORDER,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .into()
}
