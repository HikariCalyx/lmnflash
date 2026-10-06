//! The firmware-lookup UI (Mode 1): the login page and, once logged in, the
//! lookup forms for each device family together with their result views.
//!
//! The lookup state (`LookupState` and its inputs) and the update handlers
//! live in `crate`; this module only describes how that state is presented and
//! which messages its widgets send.

use iced::widget::text::Shaping;
use iced::widget::{
    button, column, container, mouse_area, pick_list, row, stack, text_editor, text_input, Column,
    Space,
};
use iced::{Alignment, Element, Fill, Length};

use crate::bulk::bulk_summary;
use crate::text;
use crate::{
    browser_login_available, firmware, l10n, protocol, BulkStatus, Click, DevicePicker,
    FastbootStatus, Labeled, LoginStatus, LookupMode, LookupResult, LookupStatus, Message,
    RetcnField, SimCount, State,
};

pub(crate) fn firmware_lookup_view(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let inner: Element<'_, Message> = match &state.login {
        LoginStatus::LoggedOut => {
            // With the scheme in our hands the left-click login runs in the
            // system browser and comes back through the protocol callback.
            let hint = if browser_login_available(state) {
                l10n.tr("login-button-hint-browser")
            } else {
                l10n.tr("login-button-hint")
            };

            column![
                text(l10n.tr("login-prompt")).size(24.0),
                mouse_area(
                    button(text(l10n.tr("login-button")).size(20.0))
                        .padding([12, 24])
                        .on_press(Message::LoginRequested(Click::Left)),
                )
                .on_right_press(Message::LoginRequested(Click::Right)),
                text(hint).size(14.0),
            ]
            .spacing(16)
            .align_x(Alignment::Center)
            .into()
        }
        LoginStatus::Fetching { .. } => {
            text(l10n.tr("login-fetching")).size(20.0).into()
        }
        LoginStatus::WebViewOpen { .. } => column![
            text(l10n.tr("login-webview-open")).size(20.0),
            button(text(l10n.tr("login-cancel"))).on_press(Message::CancelLogin),
        ]
        .spacing(12)
        .align_x(Alignment::Center)
        .into(),
        LoginStatus::WaitingForBrowser { .. } => column![
            text(l10n.tr("login-browser-waiting"))
                .size(18.0)
                .width(460)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            row![
                button(text(l10n.tr("login-open-browser"))).on_press(Message::OpenBrowser),
                button(text(l10n.tr("login-cancel"))).on_press(Message::CancelLogin),
            ]
            .spacing(8),
        ]
        .spacing(12)
        .align_x(Alignment::Center)
        .into(),
        LoginStatus::Manual { url, input, notice, .. } => {
            let placeholder = l10n.tr("login-manual-placeholder");

            let url_box = container(
                text(url.clone())
                    .size(12.0)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .width(460),
            )
            .padding(8)
            .style(container::rounded_box);

            let mut content = column![
                text(l10n.tr("login-manual-prompt")).size(20.0),
                text(l10n.tr("login-url-label")).size(14.0),
                url_box,
                row![
                    button(text(l10n.tr("login-copy-url"))).on_press(Message::CopyUrl),
                    button(text(l10n.tr("login-open-browser"))).on_press(Message::OpenBrowser),
                ]
                .spacing(8),
                text_input(&placeholder, input)
                    .on_input(Message::ManualInputChanged)
                    .on_submit(Message::SubmitManual)
                    .padding(10)
                    .width(480),
                row![
                    button(text(l10n.tr("login-submit"))).on_press(Message::SubmitManual),
                    button(text(l10n.tr("login-cancel"))).on_press(Message::CancelLogin),
                ]
                .spacing(8),
            ]
            .spacing(12)
            .align_x(Alignment::Center);

            if let Some(reason) = notice {
                content = content
                    .push(
                        text(l10n.tr("login-webview-fallback"))
                            .size(14.0)
                            .style(iced::widget::text::danger),
                    )
                    .push(text(reason.clone()).size(12.0));
            }

            if let Some(section) = protocol_section(state) {
                content = content.push(section);
            }

            content.into()
        }
        LoginStatus::Error(error) => {
            let message = l10n.tr_with_args("login-error", &[("error", error.clone())]);

            column![
                text(message).size(18.0).style(iced::widget::text::danger),
                button(text(l10n.tr("login-back"))).on_press(Message::CancelLogin),
            ]
            .spacing(12)
            .align_x(Alignment::Center)
            .into()
        }
        LoginStatus::LoggedIn { token, full_name } => {
            let mut content = column![lookup_view(state)]
                .spacing(12)
                .align_x(Alignment::Center);

            if cfg!(debug_assertions) {
                let debug = container(
                    column![
                        text(l10n.tr("debug-info")).size(16.0),
                        text(format!(
                            "{}: {}",
                            l10n.tr("debug-account"),
                            full_name.as_deref().unwrap_or("—")
                        )),
                        text(format!("{}: Bearer {}", l10n.tr("debug-token"), token)),
                        text(format!("{}: {}", l10n.tr("debug-uuid"), state.client_uuid)),
                    ]
                    .spacing(4)
                    .align_x(Alignment::Start),
                )
                .padding(12)
                .style(container::rounded_box);

                content = content.push(debug);
            }

            content.into()
        }
    };

    container(inner)
        .width(Fill)
        .height(Fill)
        .center_x(Fill)
        .center_y(Fill)
        .into()
}

/// The `softwarefix://` handler row of the Manual Login page.
///
/// Returns `None` on platforms without registry-based protocol handling, so
/// the whole section disappears there.
fn protocol_section(state: &State) -> Option<Element<'_, Message>> {
    let l10n = &state.l10n;

    let (status, action) = match &state.protocol {
        protocol::Handler::Unsupported => return None,
        protocol::Handler::None => (
            l10n.tr("login-protocol-none"),
            l10n.tr("login-protocol-switch"),
        ),
        protocol::Handler::Ours => (
            l10n.tr("login-protocol-ours"),
            l10n.tr("login-protocol-restore"),
        ),
        protocol::Handler::Other(_) => (
            l10n.tr_with_args(
                "login-protocol-current",
                &[("program", state.protocol.program().unwrap_or_default())],
            ),
            l10n.tr("login-protocol-switch"),
        ),
    };

    let mut block = column![
        text(l10n.tr("login-protocol-title")).size(14.0),
        text(status)
            .size(13.0)
            .width(460)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        button(text(action)).on_press(Message::ProtocolToggled),
    ]
    .spacing(8)
    .align_x(Alignment::Center);

    if let Some(error) = &state.protocol_error {
        let message =
            l10n.tr_with_args("login-protocol-failed", &[("error", error.clone())]);

        block = block.push(
            text(message)
                .size(12.0)
                .width(460)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    }

    Some(block.into())
}

/// The firmware lookup UI shown after a successful login.
fn lookup_view<'a>(state: &'a State) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let options: Vec<Labeled<LookupMode>> = LookupMode::ALL
        .iter()
        .map(|&mode| Labeled {
            value: mode,
            label: l10n.tr(mode.message_id()),
        })
        .collect();

    let selected = Labeled {
        value: state.lookup.mode,
        label: l10n.tr(state.lookup.mode.message_id()),
    };

    let dropdown = pick_list(options, Some(selected), |option| {
        Message::LookupModeSelected(option.value)
    })
    .text_shaping(Shaping::Advanced);

    let mode_content: Element<'_, Message> = match state.lookup.mode {
        LookupMode::RowSmartphone => {
            let placeholder = l10n.tr("lookup-imei-placeholder");
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::LookupRequested)
            };

            let content = column![
                row![
                    text(l10n.tr("lookup-imei-label")).size(16.0),
                    text_input(&placeholder, &state.lookup.imei_input)
                        .on_input(Message::ImeiInputChanged)
                        .on_submit(Message::LookupRequested)
                        .width(220),
                    lookup_button,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            push_lookup_status(state, l10n, content).into()
        }
        LookupMode::RetcnSmartphone => {
            let placeholder = l10n.tr("lookup-imei-placeholder");
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::RetcnLookupRequested)
            };

            let fastboot_fetching = matches!(
                state.lookup.retcn.fastboot_status,
                Some(FastbootStatus::Fetching)
            );
            let fill_button = if fastboot_fetching {
                button(text(l10n.tr("retcn-fill-fastboot-fetching")))
            } else {
                button(text(l10n.tr("retcn-fill-fastboot")))
                    .on_press(Message::FastbootFillRequested)
            };

            let platform_options: Vec<Labeled<firmware::Platform>> = firmware::Platform::ALL
                .iter()
                .map(|&platform| Labeled {
                    value: platform,
                    label: l10n.tr(platform.message_id()),
                })
                .collect();
            let selected_platform = Labeled {
                value: state.lookup.retcn.platform,
                label: l10n.tr(state.lookup.retcn.platform.message_id()),
            };
            let platform_dropdown: iced::widget::PickList<
                '_,
                Labeled<firmware::Platform>,
                Vec<Labeled<firmware::Platform>>,
                Labeled<firmware::Platform>,
                Message,
            > = pick_list(platform_options, Some(selected_platform), |option| {
                Message::RetcnPlatformSelected(option.value)
            })
            .text_shaping(Shaping::Advanced);

            let sim_options: Vec<Labeled<SimCount>> = SimCount::ALL
                .iter()
                .map(|&sim_count| Labeled {
                    value: sim_count,
                    label: l10n.tr(sim_count.message_id()),
                })
                .collect();
            let selected_sim = Labeled {
                value: state.lookup.retcn.sim_count,
                label: l10n.tr(state.lookup.retcn.sim_count.message_id()),
            };
            let sim_dropdown: iced::widget::PickList<
                '_,
                Labeled<SimCount>,
                Vec<Labeled<SimCount>>,
                Labeled<SimCount>,
                Message,
            > = pick_list(sim_options, Some(selected_sim), |option| {
                Message::RetcnSimCountSelected(option.value)
            })
            .text_shaping(Shaping::Advanced);

            let platform_extra: Element<'_, Message> = match state.lookup.retcn.platform {
                firmware::Platform::Qualcomm => row![
                    text(l10n.tr("retcn-fsg-label")).size(14.0),
                    text_input("", &state.lookup.retcn.fsg_version)
                        .on_input(|value| {
                            Message::RetcnFieldChanged(RetcnField::FsgVersion, value)
                        })
                        .width(240),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
                firmware::Platform::MediaTek => row![
                    text(l10n.tr("retcn-sim-label")).size(14.0),
                    sim_dropdown,
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            };

            let content = column![
                row![
                    text(l10n.tr("lookup-imei-label")).size(14.0),
                    text_input(&placeholder, &state.lookup.imei_input)
                        .on_input(Message::ImeiInputChanged)
                        .width(160),
                    text(l10n.tr("retcn-sn-label")).size(14.0),
                    text_input("", &state.lookup.retcn.serial_number)
                        .on_input(|value| {
                            Message::RetcnFieldChanged(RetcnField::SerialNumber, value)
                        })
                        .width(160),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("retcn-model-label")).size(14.0),
                    text_input("", &state.lookup.retcn.model)
                        .on_input(|value| Message::RetcnFieldChanged(RetcnField::Model, value))
                        .width(120),
                    text(l10n.tr("retcn-carrier-label")).size(14.0),
                    text_input("", &state.lookup.retcn.carrier)
                        .on_input(|value| Message::RetcnFieldChanged(RetcnField::Carrier, value))
                        .width(120),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("retcn-fingerprint-label")).size(14.0),
                    text_input("", &state.lookup.retcn.fingerprint)
                        .on_input(|value| {
                            Message::RetcnFieldChanged(RetcnField::Fingerprint, value)
                        })
                        .width(300),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("retcn-platform-label")).size(14.0),
                    platform_dropdown,
                    platform_extra,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![fill_button, lookup_button]
                    .spacing(8)
                    .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            let mut content = content;

            match &state.lookup.retcn.fastboot_status {
                Some(FastbootStatus::Filled(serial)) => {
                    content = content.push(
                        text(l10n.tr_with_args(
                            "retcn-fill-fastboot-filled",
                            &[("serial", serial.clone())],
                        ))
                        .size(13.0)
                        .style(iced::widget::text::success),
                    );
                }
                Some(FastbootStatus::Error(error)) => {
                    content = content.push(
                        text(error.clone()).size(13.0).style(iced::widget::text::danger),
                    );
                }
                _ => {}
            }

            push_lookup_status(state, l10n, content).into()
        }
        LookupMode::Tablet => {
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::TabletLookupRequested)
            };

            let content = column![
                row![
                    text(l10n.tr("tablet-sn-label")).size(16.0),
                    text_input("", &state.lookup.tablet.serial_number)
                        .on_input(Message::TabletSnChanged)
                        .on_submit(Message::TabletLookupRequested)
                        .width(220),
                    lookup_button,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            push_lookup_status(state, l10n, content).into()
        }
        LookupMode::ByModel => {
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::ModelLookupRequested)
            };

            let category_options: Vec<Labeled<firmware::Category>> = firmware::Category::ALL
                .iter()
                .map(|&category| Labeled {
                    value: category,
                    label: l10n.tr(category.message_id()),
                })
                .collect();
            let selected_category = Labeled {
                value: state.lookup.by_model.category,
                label: l10n.tr(state.lookup.by_model.category.message_id()),
            };
            let category_picker: iced::widget::PickList<
                '_,
                Labeled<firmware::Category>,
                Vec<Labeled<firmware::Category>>,
                Labeled<firmware::Category>,
                Message,
            > = pick_list(category_options, Some(selected_category), |option| {
                Message::ModelCategorySelected(option.value)
            })
            .text_shaping(Shaping::Advanced);

            let mut content = column![
                row![
                    text(l10n.tr("fw-model-name")).size(16.0),
                    text_input("", &state.lookup.by_model.model)
                        .on_input(Message::ModelInputChanged)
                        .on_submit(Message::ModelLookupRequested)
                        .width(220),
                    lookup_button,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("by-model-category-label")).size(14.0),
                    category_picker,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            // Country code only matters for tablets/smart devices
            // (phones omit it, matching `_seed_params` in motofw.py).
            if state.lookup.by_model.category != firmware::Category::Phone {
                content = content.push(
                    row![
                        text(l10n.tr("by-model-country-label")).size(14.0),
                        text_input("", &state.lookup.by_model.country_code)
                            .on_input(Message::ModelCountryChanged)
                            .width(100),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }

            // Discriminator fields the server needs for this model (shown
            // after the first request resolves `getRomMatchParams`).
            let params_ready =
                state.lookup.by_model.loaded_model == state.lookup.by_model.model.trim();
            if params_ready {
                for (index, key) in state.lookup.by_model.required.iter().enumerate() {
                    let label_id = match key.as_str() {
                        "fingerPrint" => Some("retcn-fingerprint-label"),
                        "roCarrier" => Some("retcn-carrier-label"),
                        "fsgVersion.qcom" => Some("retcn-fsg-label"),
                        "simCount" => Some("retcn-sim-label"),
                        _ => None,
                    };
                    let label = match label_id {
                        Some(id) => l10n.tr(id),
                        None => key.clone(),
                    };
                    let value = state
                        .lookup
                        .by_model
                        .values
                        .get(index)
                        .cloned()
                        .unwrap_or_default();

                    content = content.push(
                        row![
                            text(label).size(14.0),
                            text_input("", &value)
                                .on_input(move |input| Message::ModelParamChanged(index, input))
                                .width(220),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
            }

            push_lookup_status(state, l10n, content).into()
        }
        LookupMode::BulkImei => {
            let fetching = matches!(state.lookup.bulk.status, BulkStatus::Running { .. });

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::BulkLookupRequested)
            };

            let save_button = if state.lookup.bulk.rows.is_empty() {
                button(text(l10n.tr("bulk-save")))
            } else {
                button(text(l10n.tr("bulk-save"))).on_press(Message::BulkSaveRequested)
            };

            let editor = text_editor(&state.lookup.bulk.content)
                .placeholder(l10n.tr("bulk-imei-placeholder"))
                .on_action(Message::BulkEdit)
                .height(Length::Fixed(140.0))
                .width(420.0);

            let mut content = column![
                text(l10n.tr("bulk-imei-label")).size(14.0),
                editor,
                row![lookup_button, save_button]
                    .spacing(8)
                    .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            match &state.lookup.bulk.status {
                BulkStatus::Running { current, total } => {
                    content = content.push(
                        text(l10n.tr_with_args(
                            "bulk-progress",
                            &[
                                ("current", current.to_string()),
                                ("total", total.to_string()),
                            ],
                        ))
                        .size(14.0),
                    );
                }
                BulkStatus::Error(error) => {
                    content = content.push(
                        text(error.clone()).size(14.0).style(iced::widget::text::danger),
                    );
                }
                BulkStatus::Done => {
                    content = content
                        .push(text(bulk_summary(l10n, &state.lookup.bulk.rows)).size(14.0));
                }
                BulkStatus::Idle => {}
            }

            if let Some(note) = &state.lookup.bulk.save_note {
                content = content.push(match note {
                    Ok(path) => text(l10n.tr_with_args(
                        "bulk-saved",
                        &[("path", path.clone())],
                    ))
                    .size(13.0)
                    .style(iced::widget::text::success),
                    Err(error) => text(l10n.tr_with_args(
                        "bulk-save-failed",
                        &[("error", error.clone())],
                    ))
                    .size(13.0)
                    .style(iced::widget::text::danger),
                });
            }

            content.into()
        }
    };

    column![dropdown, mode_content]
        .spacing(12)
        .align_x(Alignment::Center)
        .into()
}

/// Appends the lookup status (progress, error, or result) to a column.
fn push_lookup_status<'a>(
    state: &'a State,
    l10n: &'a l10n::Bundle,
    mut content: iced::widget::Column<'a, Message>,
) -> iced::widget::Column<'a, Message> {
    match &state.lookup.status {
        LookupStatus::Idle => {}
        LookupStatus::Fetching => {
            content = content.push(text(l10n.tr("lookup-fetching")).size(14.0));
        }
        LookupStatus::Error(error) => {
            content = content.push(text(error.clone()).size(14.0).style(iced::widget::text::danger));
        }
        LookupStatus::Done(result) => {
            let view = match result {
                LookupResult::Standard(info) => firmware_info_view(l10n, info),
                LookupResult::CnTablet(info) => cn_tablet_info_view(l10n, info),
            };
            content = content.push(view);
        }
    }

    content
}

/// Displays a CN tablet lookup result, including the extraction password.
fn cn_tablet_info_view<'a>(
    l10n: &'a l10n::Bundle,
    info: &firmware::CnTabletInfo,
) -> Element<'a, Message> {
    let fields: [(&str, &str); 9] = [
        ("cn-product-name", &info.product_name),
        ("cn-product-model", &info.product_model),
        ("cn-market-name", &info.market_name),
        ("cn-mtm-compat", &info.mtm_compat),
        ("cn-latest-version", &info.latest_version),
        ("cn-id", &info.id),
        ("fw-publish-date", &info.publish_date),
        ("fw-file-name", &info.file_name),
        ("fw-file-size", &info.file_size),
    ];

    let copy_uri_button = if info.download_url.is_empty() {
        button(text(l10n.tr("fw-copy-uri")))
    } else {
        button(text(l10n.tr("fw-copy-uri")))
            .on_press(Message::CopyDownloadUri(info.download_url.clone()))
    };

    let copy_password_button = button(text(l10n.tr("tablet-copy-password")))
        .on_press(Message::CopyCnPassword(info.unzip_password.clone()));

    let buttons = row![copy_uri_button, copy_password_button].spacing(8);

    container(
        column!(
            column(fields.iter().map(|(id, value)| {
                let label = l10n.tr(id);
                let value = if value.is_empty() { "—" } else { value };

                text(format!("{label}: {value}")).size(13.0).into()
            }))
            .spacing(4)
            .align_x(Alignment::Start),
            buttons,
        )
        .spacing(8)
        .align_x(Alignment::Start),
    )
    .padding(12)
    .width(520)
    .style(container::rounded_box)
    .into()
}

/// Displays the fields of a firmware lookup result.
fn firmware_info_view<'a>(
    l10n: &'a l10n::Bundle,
    info: &firmware::FirmwareInfo,
) -> Element<'a, Message> {
    let fields: [(&str, &str); 11] = [
        ("fw-market-name", &info.market_name),
        ("fw-model-name", &info.model_name),
        ("fw-sale-model", &info.sale_model),
        ("fw-carrier", &info.carrier),
        ("fw-publish-date", &info.publish_date),
        ("fw-file-name", &info.file_name),
        ("fw-file-size", &info.file_size),
        ("fw-rom-id", &info.rom_id),
        ("fw-rom-match-id", &info.rom_match_id),
        ("fw-fingerprint", &info.fingerprint),
        ("fw-comments", &info.comments),
    ];

    let copy_uri_button = if info.rom_uri.is_empty() {
        button(text(l10n.tr("fw-copy-uri")))
    } else {
        button(text(l10n.tr("fw-copy-uri")))
            .on_press(Message::CopyDownloadUri(info.rom_uri.clone()))
    };

    let copy_tool_button = if info.tool_uri.is_empty() {
        button(text(l10n.tr("fw-copy-tool")))
    } else {
        button(text(l10n.tr("fw-copy-tool")))
            .on_press(Message::CopyToolUri(info.tool_uri.clone()))
    };

    let copy_raw_button = button(text(l10n.tr("fw-copy-raw")))
        .on_press(Message::CopyRawJson(info.raw_json.clone()));

    let buttons = row![copy_uri_button, copy_tool_button, copy_raw_button].spacing(8);

    container(
        column!(
            column(fields.iter().map(|(id, value)| {
                let label = l10n.tr(id);
                let value = if value.is_empty() { "—" } else { value };

                text(format!("{label}: {value}")).size(13.0).into()
            }))
            .spacing(4)
            .align_x(Alignment::Start),
            buttons,
        )
        .spacing(8)
        .align_x(Alignment::Start),
    )
    .padding(12)
    .width(520)
    .style(container::rounded_box)
    .into()
}

/// Device picker modal: shown when more than one fastboot device is connected.
/// Clicks on the dimmed backdrop cancel the selection.
pub(crate) fn device_picker(state: &State) -> Option<Element<'_, Message>> {
    let DevicePicker::Open(devices) = &state.lookup.retcn.device_picker else {
        return None;
    };

    let device_buttons = Column::with_children(devices.iter().map(|device| {
        button(text(device.label()))
            .width(Fill)
            .on_press(Message::FastbootDeviceSelected(device.serial.clone()))
            .into()
    }))
    .spacing(8);

    let card = container(
        column![
            text(state.l10n.tr("retcn-pick-device-title")).size(18.0),
            device_buttons,
            button(text(state.l10n.tr("login-cancel")))
                .on_press(Message::FastbootDevicePickerCancelled),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .padding(16)
    .width(340)
    .style(container::rounded_box);

    let backdrop =
        mouse_area(Space::new(Fill, Fill)).on_press(Message::FastbootDevicePickerCancelled);

    Some(
        stack![
            backdrop,
            container(card)
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill),
        ]
        .into(),
    )
}
