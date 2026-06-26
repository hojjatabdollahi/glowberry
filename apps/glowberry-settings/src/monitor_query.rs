// SPDX-License-Identifier: MPL-2.0

use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MonitorQueryError {
    #[error("failed to run cosmic-randr: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("invalid utf8 output: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("failed to parse KDL: {0}")]
    Kdl(#[from] kdl::KdlError),
    #[error("no current mode for output {0}")]
    #[allow(dead_code)]
    NoCurrentMode(String),
}

#[derive(Debug, Clone)]
pub struct MonitorGeometry {
    pub name: String,
    pub position: (i32, i32),
    pub logical_size: (u32, u32),
    pub physical_size: (u32, u32),
    pub scale: f64,
    pub bezel: glowberry_config::extend::Bezel,
    /// Monitor model name from EDID (e.g. "LG HDR 4K"), if reported.
    pub model: Option<String>,
    /// Stable EDID-derived identity (make|model|serial), if reported. Same
    /// physical monitor yields the same value regardless of which port it's on.
    pub edid: Option<String>,
}

impl MonitorGeometry {
    /// Key used to store per-display settings (bezels). Uses the EDID identity
    /// so the same monitor keeps its settings across ports; falls back to the
    /// connector name when EDID info is unavailable.
    pub fn bezel_key(&self) -> String {
        self.edid.clone().unwrap_or_else(|| self.name.clone())
    }

    /// Human-friendly label: the monitor model with the current connector in
    /// parentheses (e.g. "LG HDR 4K (DP-6)"); just the connector if no model.
    pub fn display_label(&self) -> String {
        match &self.model {
            Some(model) if !model.is_empty() => format!("{model} ({})", self.name),
            _ => self.name.clone(),
        }
    }
}

pub async fn query_monitors() -> Result<Vec<MonitorGeometry>, MonitorQueryError> {
    let output = tokio::task::spawn_blocking(|| {
        Command::new("cosmic-randr")
            .args(["list", "--kdl"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
    })
    .await
    .expect("tokio task panicked")?;

    let stdout = std::str::from_utf8(&output.stdout)?;
    let document: kdl::KdlDocument = stdout.parse()?;

    let mut monitors = Vec::new();

    for node in document.nodes() {
        if node.name().value() != "output" {
            continue;
        }

        let mut entries = node.entries().iter();

        let Some(name) = entries.next().and_then(|e| e.value().as_string()) else {
            continue;
        };

        let mut enabled = true;
        for entry in entries {
            if let Some(entry_name) = entry.name()
                && entry_name.value() == "enabled"
                && let Some(val) = entry.value().as_bool()
            {
                enabled = val;
            }
        }

        if !enabled {
            continue;
        }

        let Some(children) = node.children() else {
            continue;
        };

        let mut position = (0i32, 0i32);
        let mut scale = 1.0f64;
        let mut current_mode_size: Option<(u32, u32)> = None;
        let mut is_rotated = false;
        let mut make: Option<String> = None;
        let mut model: Option<String> = None;
        let mut serial: Option<String> = None;

        for child in children.nodes() {
            match child.name().value() {
                "description" => {
                    for entry in child.entries() {
                        match entry.name().map(|n| n.value()) {
                            Some("make") => {
                                make = entry.value().as_string().map(str::to_owned);
                            }
                            Some("model") => {
                                model = entry.value().as_string().map(str::to_owned);
                            }
                            _ => {}
                        }
                    }
                }
                "serial_number" => {
                    if let Some(entry) = child.entries().first()
                        && let Some(s) = entry.value().as_string()
                    {
                        serial = Some(s.to_owned());
                    }
                }
                "position" => {
                    if let [x, y, ..] = child.entries() {
                        position = (
                            x.value().as_integer().unwrap_or_default() as i32,
                            y.value().as_integer().unwrap_or_default() as i32,
                        );
                    }
                }
                "scale" => {
                    if let Some(entry) = child.entries().first()
                        && let Some(s) = entry.value().as_float()
                    {
                        scale = s;
                    }
                }
                "transform" => {
                    if let Some(entry) = child.entries().first()
                        && let Some(t) = entry.value().as_string()
                    {
                        is_rotated =
                            matches!(t, "rotate90" | "rotate270" | "flipped90" | "flipped270");
                    }
                }
                "modes" => {
                    if let Some(modes_children) = child.children() {
                        for mode_node in modes_children.nodes() {
                            if mode_node.name().value() != "mode" {
                                continue;
                            }
                            let is_current = mode_node
                                .entries()
                                .iter()
                                .skip(3)
                                .any(|e| e.name().map(|n| n.value()) == Some("current"));

                            if is_current && let [w, h, ..] = mode_node.entries() {
                                current_mode_size = Some((
                                    w.value().as_integer().unwrap_or_default() as u32,
                                    h.value().as_integer().unwrap_or_default() as u32,
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let Some(mode_size) = current_mode_size else {
            tracing::warn!(name, "skipping output with no current mode");
            continue;
        };

        let (phys_w, phys_h) = if is_rotated {
            (mode_size.1, mode_size.0)
        } else {
            mode_size
        };

        let logical_w = (phys_w as f64 / scale).round() as u32;
        let logical_h = (phys_h as f64 / scale).round() as u32;

        // Build a stable EDID identity when any descriptor is present.
        let edid = if make.is_some() || model.is_some() || serial.is_some() {
            Some(format!(
                "{}|{}|{}",
                make.as_deref().unwrap_or(""),
                model.as_deref().unwrap_or(""),
                serial.as_deref().unwrap_or(""),
            ))
        } else {
            None
        };

        monitors.push(MonitorGeometry {
            name: name.to_owned(),
            position,
            logical_size: (logical_w, logical_h),
            physical_size: (phys_w, phys_h),
            scale,
            bezel: glowberry_config::extend::Bezel::default(),
            model,
            edid,
        });
    }

    Ok(monitors)
}
