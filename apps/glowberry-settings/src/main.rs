// SPDX-License-Identifier: MPL-2.0

//! GlowBerry Settings - Configuration application for GlowBerry background service

mod app;
mod i18n;
mod pages;
mod shader_params;
mod widgets;

use app::GlowBerrySettings;

fn main() -> cosmic::iced::Result {
    // Get the system's preferred languages and initialize i18n
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();
    i18n::init(&requested_languages);

    // Settings for configuring the application window and iced runtime
    let settings = cosmic::app::Settings::default()
        .size_limits(cosmic::iced::Limits::NONE.min_width(400.0).min_height(300.0));

    // Start the application
    cosmic::app::run::<GlowBerrySettings>(settings, ())
}
