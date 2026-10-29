use crate::{
    App, Message,
    camera::{AUTO_WB, BRIGHTNESS, CONTRAST, Control, SATURATION, TEMPERATURE},
};
use iced::{
    Border, Color, Element, Fill, Font, Length, Shadow, Theme, Vector, font,
    widget::{
        Column, Space, button, checkbox, column, container, image, pick_list, row, scrollable,
        slider, text,
    },
};

const INK: Color = Color::from_rgb8(26, 40, 49);
const MUTED: Color = Color::from_rgb8(106, 120, 130);
const ACCENT: Color = Color::from_rgb8(13, 126, 117);
const BORDER: Color = Color::from_rgb8(225, 231, 234);
const PRESETS: [(&str, i32); 5] = [
    ("Tungsten", 3200),
    ("Fluorescent", 4200),
    ("Daylight", 5500),
    ("Cloudy", 6500),
    ("Shade", 7500),
];
pub fn theme() -> Theme {
    Theme::custom(
        "Camera Control",
        iced::theme::Palette {
            background: Color::from_rgb8(244, 247, 248),
            text: INK,
            primary: ACCENT,
            success: ACCENT,
            warning: Color::from_rgb8(181, 120, 37),
            danger: Color::from_rgb8(184, 58, 58),
        },
    )
}
fn strong<'a>(value: impl Into<std::borrow::Cow<'a, str>>, size: u32) -> iced::widget::Text<'a> {
    text(value.into()).size(size).font(Font {
        family: font::Family::Name("Noto Sans"),
        weight: font::Weight::Semibold,
        ..Font::DEFAULT
    })
}
fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(23)
        .width(Fill)
        .style(|_| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 16.0.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba8(29, 47, 61, 0.035),
                offset: Vector::new(0.0, 3.0),
                blur_radius: 12.0,
            },
            ..Default::default()
        })
        .into()
}
fn section_label(value: &str) -> iced::widget::Text<'_> {
    text(value).size(11).color(MUTED)
}
fn chip<'a>(value: &'a str, active: bool) -> Element<'a, Message> {
    container(
        text(value)
            .size(12)
            .color(if active { ACCENT } else { MUTED }),
    )
    .padding([6, 11])
    .style(move |_| container::Style {
        background: Some(
            if active {
                Color::from_rgb8(231, 246, 242)
            } else {
                Color::from_rgb8(241, 244, 246)
            }
            .into(),
        ),
        border: Border {
            radius: 30.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}
pub fn view(app: &App) -> Element<'_, Message> {
    let profile = app.profile();
    let desired = &profile.desired;
    let automatic = app
        .controls
        .iter()
        .find(|c| c.id == AUTO_WB)
        .is_some_and(|control| desired.get(&AUTO_WB).copied().unwrap_or(control.value) != 0);
    let title = column![
        section_label("CAMERA CONTROL"),
        strong("Camera settings", 28)
    ]
    .spacing(8);
    let mut tray_button = button("Hide to tray").padding([10, 15]).style(button::text);
    if app.tray_ready {
        tray_button = tray_button.on_press(Message::Hide);
    }
    let header = row![title, Space::new().width(Fill), tray_button].align_y(iced::Center);
    let online = app.selected.is_some() && app.error.is_none() && !app.controls.is_empty();
    let device = card(
        column![
            row![
                section_label("YOUR CAMERA"),
                Space::new().width(Fill),
                chip(
                    if online {
                        "Connected"
                    } else {
                        "Waiting for camera"
                    },
                    online
                )
            ]
            .align_y(iced::Center),
            row![
                pick_list(app.cameras.clone(), app.selected.clone(), Message::Selected)
                    .placeholder("Connect or select a camera")
                    .width(Fill)
                    .padding(11)
                    .style(picker_style),
                button("Refresh")
                    .on_press(Message::Refresh)
                    .padding([11, 16])
                    .style(quiet_button)
            ]
            .spacing(12),
        ]
        .spacing(12),
    );

    let mut balance = column![
        section_label("01 / LIGHT"),
        strong("White balance", 22),
        text("Choose the light around you, then fine-tune.")
            .size(13)
            .color(MUTED),
    ]
    .spacing(12);
    if app.controls.iter().any(|c| c.id == AUTO_WB) {
        balance = balance.push(
            row![
                button("Automatic")
                    .padding([8, 17])
                    .style(if automatic {
                        accent_button
                    } else {
                        quiet_button
                    })
                    .on_press(Message::Value(AUTO_WB, 1)),
                button("Manual")
                    .padding([8, 17])
                    .style(if !automatic {
                        accent_button
                    } else {
                        quiet_button
                    })
                    .on_press(Message::Value(AUTO_WB, 0)),
            ]
            .spacing(8),
        );
    }
    if let Some(control) = app.controls.iter().find(|c| c.id == TEMPERATURE) {
        let value = control.normalize(desired.get(&TEMPERATURE).copied().unwrap_or(control.value));
        let active_name = if automatic {
            "Camera is adjusting"
        } else {
            PRESETS
                .iter()
                .find(|(_, k)| control.normalize(*k) == value)
                .map(|(name, _)| *name)
                .unwrap_or("Custom temperature")
        };
        balance = balance.push(
            row![
                strong(format!("{value}"), 40),
                text("K").size(20).color(MUTED),
                Space::new().width(Fill),
                chip(active_name, !automatic)
            ]
            .spacing(6)
            .align_y(iced::Center),
        );
        if !automatic {
            balance = balance.push(
                slider(control.minimum..=control.maximum, value, |v| {
                    Message::Value(TEMPERATURE, v)
                })
                .step(control.step.max(100)),
            );
        } else {
            balance = balance.push(
                text("Select a preset to switch to manual white balance.")
                    .size(12)
                    .color(MUTED),
            );
        }
        let spectrum = row![
            swatch(Color::from_rgb8(237, 165, 107)),
            swatch(Color::from_rgb8(242, 197, 143)),
            swatch(Color::from_rgb8(242, 223, 186)),
            swatch(Color::from_rgb8(229, 236, 228)),
            swatch(Color::from_rgb8(193, 219, 238)),
            swatch(Color::from_rgb8(146, 187, 222)),
        ]
        .spacing(3);
        balance = balance.push(spectrum).push(row![
            text("Warm light").size(11).color(MUTED),
            Space::new().width(Fill),
            text("Cool light").size(11).color(MUTED)
        ]);
        // Wrap at three presets so the panel remains comfortable on smaller screens.
        let mut presets = Column::new().spacing(7);
        for group in PRESETS.chunks(3) {
            let mut buttons = row![].spacing(7);
            for &(name, kelvin) in group {
                let selected = !automatic && value == control.normalize(kelvin);
                let label = column![
                    text(name).size(12),
                    text(format!("{} K", control.normalize(kelvin))).size(11)
                ]
                .spacing(3);
                let mut preset =
                    button(label)
                        .padding([9, 12])
                        .width(Length::Fill)
                        .style(if selected {
                            accent_button
                        } else {
                            quiet_button
                        });
                if (control.minimum..=control.maximum).contains(&kelvin) {
                    preset = preset.on_press(Message::Preset(kelvin));
                }
                buttons = buttons.push(preset);
            }
            presets = presets.push(buttons);
        }
        balance = balance.push(Space::new().height(2)).push(presets);
    } else {
        balance = balance.push(
            text("This camera does not expose a temperature control.")
                .size(13)
                .color(MUTED),
        );
    }

    let mut tuning = column![
        section_label("02 / COLOR"),
        strong("Image tuning", 22),
        text("Small changes go a long way.").size(13).color(MUTED),
        Space::new().height(8)
    ]
    .spacing(13);
    let mut tuning_count = 0;
    for id in [BRIGHTNESS, CONTRAST, SATURATION] {
        if let Some(control) = app.controls.iter().find(|c| c.id == id) {
            tuning_count += 1;
            tuning = tuning.push(control_row(
                control,
                desired.get(&id).copied().unwrap_or(control.value),
            ));
        }
    }
    if tuning_count == 0 {
        tuning = tuning.push(
            text("No color sliders are exposed by this camera.")
                .size(13)
                .color(MUTED),
        );
    }
    let mut preview_toggle = checkbox(app.preview_enabled)
        .label("Enable preview")
        .size(17)
        .text_size(13);
    if app.selected.is_some() && (app.preview.is_none() || app.preview_enabled) {
        preview_toggle = preview_toggle.on_toggle(Message::Preview);
    }
    let mut preview_content = column![
        row![
            section_label("LIVE VIEW"),
            Space::new().width(Fill),
            preview_toggle
        ]
        .align_y(iced::Center)
    ]
    .spacing(13);
    let resolutions: Vec<_> = app
        .preview_modes
        .iter()
        .map(|mode| mode.resolution)
        .collect();
    let mut frame_rates = app
        .preview_modes
        .iter()
        .find(|mode| mode.resolution == profile.preview.resolution)
        .map(|mode| mode.frame_rates.clone())
        .unwrap_or_default();
    frame_rates.extend([15, 24, 25, 30, 50, 60]);
    frame_rates.retain(|rate| (1..=60).contains(rate));
    frame_rates.sort_unstable();
    frame_rates.dedup();
    let mut mirror = checkbox(profile.preview.mirror)
        .label("Mirror preview")
        .size(17)
        .text_size(13);
    if app.selected.is_some() {
        mirror = mirror.on_toggle(Message::PreviewMirror);
    }
    preview_content = preview_content.push(
        row![
            column![
                text("Resolution").size(11).color(MUTED),
                pick_list(
                    resolutions,
                    Some(profile.preview.resolution),
                    Message::PreviewResolution
                )
                .padding([7, 11])
                .style(picker_style)
            ]
            .spacing(5),
            column![
                text("Frame rate (fps)").size(11).color(MUTED),
                pick_list(
                    frame_rates,
                    Some(profile.preview.fps),
                    Message::PreviewFrameRate
                )
                .padding([7, 11])
                .style(picker_style)
            ]
            .spacing(5),
            Space::new().width(Fill),
            mirror,
        ]
        .spacing(16)
        .align_y(iced::Center),
    );
    if let Some(info) = &app.preview_info
        && app.preview_enabled
    {
        preview_content = preview_content.push(
            text(format!(
                "{} × {} · {} fps requested · {:.0} fps camera · {:.1} fps preview",
                info.width, info.height, profile.preview.fps, info.camera_fps, info.measured_fps
            ))
            .size(11)
            .color(MUTED),
        );
    }
    if app.preview_enabled {
        let frame: Element<'_, Message> = if let Some(frame) = &app.preview_frame {
            image(frame.clone())
                .width(Fill)
                .height(280)
                .content_fit(iced::ContentFit::Contain)
                .into()
        } else {
            container(text("Opening camera…").size(14).color(MUTED))
                .center_x(Fill)
                .center_y(180)
                .into()
        };
        preview_content =
            preview_content.push(container(frame).width(Fill).style(|_| container::Style {
                background: Some(Color::from_rgb8(19, 28, 36).into()),
                border: Border {
                    radius: 10.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }));
    } else {
        preview_content = preview_content.push(
            text(if app.preview.is_some() {
                "Releasing camera…"
            } else {
                "Preview is off. Your camera is available to other apps."
            })
            .size(12)
            .color(MUTED),
        );
    }
    if let Some(error) = &app.preview_error {
        preview_content =
            preview_content.push(text(error).size(12).color(Color::from_rgb8(159, 71, 38)));
    }
    let preview_panel = card(preview_content);
    let panels = row![
        container(card(balance)).width(Length::FillPortion(3)),
        container(card(tuning)).width(Length::FillPortion(2))
    ]
    .spacing(18);
    let mut watch = checkbox(profile.watch)
        .label("Watch camera status and enforce color settings")
        .size(17)
        .text_size(13);
    if !app.controls.is_empty() {
        watch = watch.on_toggle(Message::Watch);
    }
    let startup = checkbox(app.start_at_login)
        .label("Start at login")
        .size(17)
        .text_size(13)
        .on_toggle(Message::StartAtLogin);
    let behavior = card(
        column![
            row![
                section_label("03 / BACKGROUND"),
                Space::new().width(Fill),
                chip(
                    if profile.watch {
                        "Watching"
                    } else {
                        "On demand"
                    },
                    profile.watch
                )
            ]
            .align_y(iced::Center),
            watch,
            row![
                startup,
                Space::new().width(Fill),
                text(if app.tray_ready {
                    "Closing the window keeps the app in the tray."
                } else {
                    "Tray connection is starting…"
                })
                .size(12)
                .color(MUTED)
            ]
            .spacing(18)
            .align_y(iced::Center),
        ]
        .spacing(13),
    );
    let mut content = column![header, device, preview_panel, panels, behavior].spacing(18);
    for error in [&app.error, &app.config_error, &app.desktop_error]
        .into_iter()
        .flatten()
    {
        content = content.push(
            container(text(error).size(13).color(Color::from_rgb8(159, 71, 38)))
                .padding(13)
                .width(Fill)
                .style(|_| container::Style {
                    background: Some(Color::from_rgb8(255, 243, 230).into()),
                    border: Border {
                        radius: 10.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    let footer = row![
        text("Saved per camera").size(11).color(MUTED),
        Space::new().width(Fill),
        button("Quit")
            .on_press(Message::Quit)
            .style(button::text)
            .padding([3, 9])
    ]
    .align_y(iced::Center);
    content = content.push(footer);
    container(scrollable(
        container(container(content).max_width(1100).padding(28)).center_x(Fill),
    ))
    .width(Fill)
    .height(Fill)
    .into()
}
fn swatch(color: Color) -> Element<'static, Message> {
    container(Space::new().height(4))
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
fn control_row(control: &Control, value: i32) -> Element<'static, Message> {
    let id = control.id;
    let label = row![
        text(crate::label(id)).size(13),
        Space::new().width(Fill),
        strong(value.to_string(), 14)
    ];
    let input: Element<'static, Message> = if control.inactive {
        text("Managed by camera").size(12).color(MUTED).into()
    } else {
        slider(
            control.minimum..=control.maximum,
            control.normalize(value),
            move |v| Message::Value(id, v),
        )
        .step(control.step)
        .into()
    };
    column![label, input, Space::new().height(7)]
        .spacing(9)
        .into()
}

fn quiet_button(_theme: &Theme, status: button::Status) -> button::Style {
    let disabled = matches!(status, button::Status::Disabled);
    let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(
            if hover {
                Color::from_rgb8(232, 245, 241)
            } else {
                Color::from_rgb8(247, 249, 250)
            }
            .into(),
        ),
        text_color: if disabled { MUTED } else { INK },
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}
fn accent_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.border.radius = 8.0.into();
    style
}
fn picker_style(_theme: &Theme, _status: pick_list::Status) -> pick_list::Style {
    pick_list::Style {
        text_color: INK,
        placeholder_color: MUTED,
        handle_color: MUTED,
        background: Color::from_rgb8(247, 249, 250).into(),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
    }
}
