// SPDX-License-Identifier: MPL-2.0

//! UPower D-Bus client for monitoring power state.
//!
//! This module provides async monitoring of:
//! - Battery on/off state (OnBattery property)
//! - Battery percentage (via DisplayDevice)
//! - Lid closed state (LidIsClosed property)

use futures::StreamExt;
use tokio::sync::watch;
use zbus::{Connection, proxy};

/// UPower D-Bus proxy for the main UPower interface.
#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    /// Whether the system is running on battery power.
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;

    /// Whether the lid is closed.
    #[zbus(property)]
    fn lid_is_closed(&self) -> zbus::Result<bool>;

    /// Whether the system has a lid.
    #[zbus(property)]
    fn lid_is_present(&self) -> zbus::Result<bool>;

    /// Get the display device object path.
    fn get_display_device(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

/// UPower Device D-Bus proxy for battery information.
#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
trait UPowerDevice {
    /// Battery percentage (0-100).
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    /// Device state (charging, discharging, etc.).
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

/// Current power state snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerState {
    /// Whether the system is running on battery power.
    pub on_battery: bool,
    /// Battery percentage (0-100), or None if no battery.
    pub battery_percentage: Option<f64>,
    /// Whether the lid is closed (always false if no lid).
    pub lid_is_closed: bool,
}

impl Default for PowerState {
    fn default() -> Self {
        Self {
            on_battery: false,
            battery_percentage: None,
            lid_is_closed: false,
        }
    }
}

/// Handle to the power monitor, providing access to current state.
#[derive(Clone)]
pub struct PowerMonitorHandle {
    rx: watch::Receiver<PowerState>,
}

impl PowerMonitorHandle {
    /// Get the current power state.
    pub fn current(&self) -> PowerState {
        *self.rx.borrow()
    }

    /// Wait for the power state to change.
    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.rx.changed().await
    }
}

/// Power monitor that watches UPower D-Bus signals.
pub struct PowerMonitor {
    tx: watch::Sender<PowerState>,
    handle: PowerMonitorHandle,
}

impl PowerMonitor {
    /// Create a new power monitor.
    /// 
    /// Returns the monitor and a handle that can be used to query the current state.
    pub fn new() -> (Self, PowerMonitorHandle) {
        let (tx, rx) = watch::channel(PowerState::default());
        let handle = PowerMonitorHandle { rx };
        (Self { tx, handle: handle.clone() }, handle)
    }

    /// Get a handle to query the current power state.
    pub fn handle(&self) -> PowerMonitorHandle {
        self.handle.clone()
    }

    /// Start monitoring power state changes.
    /// 
    /// This spawns a tokio task that monitors UPower D-Bus signals and updates
    /// the power state accordingly. The task runs until the connection is lost
    /// or the monitor is dropped.
    pub async fn start(self) -> zbus::Result<()> {
        let connection = Connection::system().await?;
        let upower = UPowerProxy::new(&connection).await?;

        // Get initial state
        let on_battery = upower.on_battery().await.unwrap_or(false);
        let lid_is_closed = upower.lid_is_closed().await.unwrap_or(false);
        
        // Get battery percentage from display device
        let battery_percentage = match upower.get_display_device().await {
            Ok(path) => {
                let device = UPowerDeviceProxy::builder(&connection)
                    .path(path)?
                    .build()
                    .await?;
                device.percentage().await.ok()
            }
            Err(_) => None,
        };

        // Send initial state
        let initial_state = PowerState {
            on_battery,
            battery_percentage,
            lid_is_closed,
        };
        let _ = self.tx.send(initial_state);
        tracing::info!(?initial_state, "Power monitor started");

        // Clone what we need for the monitoring task
        let tx = self.tx.clone();
        
        // Spawn monitoring task
        tokio::spawn(async move {
            if let Err(e) = monitor_loop(connection, tx).await {
                tracing::error!(?e, "Power monitor error");
            }
        });

        Ok(())
    }
}

async fn monitor_loop(
    connection: Connection,
    tx: watch::Sender<PowerState>,
) -> zbus::Result<()> {
    let upower = UPowerProxy::new(&connection).await?;
    
    // Get display device for battery monitoring
    let display_device_path = upower.get_display_device().await.ok();
    let display_device = if let Some(ref path) = display_device_path {
        UPowerDeviceProxy::builder(&connection)
            .path(path.clone())?
            .build()
            .await
            .ok()
    } else {
        None
    };

    // Subscribe to property changes
    let mut on_battery_stream = upower.receive_on_battery_changed().await;
    let mut lid_closed_stream = upower.receive_lid_is_closed_changed().await;
    
    // Subscribe to battery percentage changes if we have a display device
    let mut percentage_stream = if let Some(ref device) = display_device {
        Some(device.receive_percentage_changed().await)
    } else {
        None
    };

    loop {
        tokio::select! {
            Some(change) = async { on_battery_stream.next().await } => {
                if let Ok(on_battery) = change.get().await {
                    tx.send_modify(|state| {
                        state.on_battery = on_battery;
                    });
                    tracing::debug!(on_battery, "Battery state changed");
                }
            }
            Some(change) = async { lid_closed_stream.next().await } => {
                if let Ok(lid_is_closed) = change.get().await {
                    tx.send_modify(|state| {
                        state.lid_is_closed = lid_is_closed;
                    });
                    tracing::debug!(lid_is_closed, "Lid state changed");
                }
            }
            Some(change) = async { 
                if let Some(ref mut stream) = percentage_stream {
                    stream.next().await
                } else {
                    std::future::pending().await
                }
            } => {
                if let Ok(percentage) = change.get().await {
                    tx.send_modify(|state| {
                        state.battery_percentage = Some(percentage);
                    });
                    tracing::debug!(percentage, "Battery percentage changed");
                }
            }
            else => {
                tracing::warn!("All power monitoring streams ended");
                break;
            }
        }
    }

    Ok(())
}

/// Start a background power monitor and return a handle.
/// 
/// This is a convenience function that creates a monitor and starts it
/// on a new tokio runtime if one isn't already running.
pub fn start_power_monitor() -> Option<PowerMonitorHandle> {
    // Create a new tokio runtime for the power monitor
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;

    let (monitor, handle) = PowerMonitor::new();
    
    // Spawn the monitor on a separate thread with its own runtime
    std::thread::spawn(move || {
        rt.block_on(async {
            if let Err(e) = monitor.start().await {
                tracing::error!(?e, "Failed to start power monitor");
            }
            // Keep the runtime alive
            std::future::pending::<()>().await
        });
    });

    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_state_default() {
        let state = PowerState::default();
        assert!(!state.on_battery);
        assert!(state.battery_percentage.is_none());
        assert!(!state.lid_is_closed);
    }
}
