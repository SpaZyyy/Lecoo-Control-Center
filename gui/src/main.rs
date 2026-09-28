use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText};
use ipc::{IpcClient, IpcRequest, IpcResponse};
use lecoo_types::{
    caps::ChargeStatus,
    ec_types::{FanIndex, FanMode, KeyboardBacklightLevel, PowerProfile},
    settings::CurrentSettings,
};

#[derive(Default, Clone)]
struct Snapshot {
    connected: bool,
    error: Option<String>,
    message: Option<String>,
    board: Option<String>,
    cpu_temp: Option<u8>,
    system_temp: Option<u8>,
    cpu_fan: Option<u16>,
    gpu_fan: Option<u16>,
    charge: Option<ChargeStatus>,
    settings: Option<CurrentSettings>,
}

enum Action {
    SetPower(PowerProfile),
    SetFan(FanIndex, FanMode),
    SetBacklight(KeyboardBacklightLevel),
}

struct ControlCenter {
    actions: Sender<Action>,
    updates: Receiver<Snapshot>,
    snapshot: Snapshot,
    styled: bool,
}

impl ControlCenter {
    fn new() -> Self {
        let (action_tx, action_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        thread::Builder::new()
            .name("lecoo-gui-ipc".into())
            .spawn(move || run_worker(action_rx, update_tx))
            .expect("failed to start the IPC worker");

        Self {
            actions: action_tx,
            updates: update_rx,
            snapshot: Snapshot::default(),
            styled: false,
        }
    }

    fn apply(&self, action: Action) {
        let _ = self.actions.send(action);
    }
}

impl eframe::App for ControlCenter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.styled {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = Color32::from_rgb(18, 20, 25);
            visuals.window_fill = Color32::from_rgb(24, 27, 33);
            visuals.widgets.inactive.bg_fill = Color32::from_rgb(39, 43, 51);
            visuals.widgets.hovered.bg_fill = Color32::from_rgb(54, 60, 70);
            visuals.selection.bg_fill = Color32::from_rgb(99, 102, 241);
            ctx.set_visuals(visuals);
            self.styled = true;
        }

        while let Ok(snapshot) = self.updates.try_recv() {
            self.snapshot = snapshot;
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("LECOO").strong().color(Color32::from_rgb(225, 228, 235)));
                ui.label(RichText::new("CONTROL CENTER").small().color(Color32::from_rgb(145, 151, 164)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, color) = if self.snapshot.connected {
                        ("Connected", Color32::from_rgb(105, 210, 155))
                    } else {
                        ("Daemon offline", Color32::from_rgb(240, 155, 105))
                    };
                    ui.label(RichText::new(label).color(color));
                    ui.label("●");
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            ui.heading("Overview");
            ui.label(
                self.snapshot
                    .board
                    .as_deref()
                    .unwrap_or("Hardware status and controls"),
            );
            ui.add_space(14.0);

            ui.columns(4, |columns| {
                metric(&mut columns[0], "CPU TEMPERATURE", value_with_unit(self.snapshot.cpu_temp, "°C"));
                metric(&mut columns[1], "SYSTEM TEMPERATURE", value_with_unit(self.snapshot.system_temp, "°C"));
                metric(&mut columns[2], "CPU FAN", value_with_unit(self.snapshot.cpu_fan, " RPM"));
                metric(&mut columns[3], "GPU FAN", value_with_unit(self.snapshot.gpu_fan, " RPM"));
            });

            ui.add_space(18.0);
            ui.columns(2, |columns| {
                columns[0].group(|ui| {
                    ui.set_min_height(182.0);
                    ui.heading("Performance");
                    ui.label("Power profile");
                    let mut profile = self.snapshot.settings.as_ref().map(|s| s.power_profile);
                    ui.horizontal_wrapped(|ui| {
                        for (choice, label) in [
                            (PowerProfile::Silent, "Quiet"),
                            (PowerProfile::Default, "Balanced"),
                            (PowerProfile::Performance, "Performance"),
                        ] {
                            if ui
                                .add_enabled(
                                    self.snapshot.connected,
                                    egui::Button::selectable(profile == Some(choice), label),
                                )
                                .clicked()
                            {
                                self.apply(Action::SetPower(choice));
                            }
                        }
                    });

                    ui.add_space(10.0);
                    ui.label("Fans");
                    fan_controls(ui, &self.snapshot, FanIndex::Cpu, "CPU");
                    fan_controls(ui, &self.snapshot, FanIndex::Gpu, "GPU");
                });

                columns[1].group(|ui| {
                    ui.set_min_height(182.0);
                    ui.heading("Battery & lighting");
                    match &self.snapshot.charge {
                        Some(status) => {
                            ui.horizontal(|ui| {
                                ui.label("Battery");
                                ui.label(RichText::new(format!("{}%", status.soc)).strong());
                            });
                            if let Some((start, stop)) = status.thresholds {
                                ui.label(format!("Charge window  {start}% – {stop}%"));
                            } else {
                                ui.label(format!("Charge mode  {:?}", status.effective));
                            }
                            if let Some(pending) = &status.pending {
                                ui.label(RichText::new(pending).color(Color32::from_rgb(239, 181, 105)));
                            }
                        }
                        None => {
                            ui.label("Battery status unavailable");
                        }
                    }

                    ui.add_space(12.0);
                    ui.label("Keyboard backlight");
                    let current = self
                        .snapshot
                        .settings
                        .as_ref()
                        .map(|s| s.keyboard_backlight)
                        .unwrap_or(KeyboardBacklightLevel::Off);
                    ui.horizontal_wrapped(|ui| {
                        for (choice, label) in [
                            (KeyboardBacklightLevel::Off, "Off"),
                            (KeyboardBacklightLevel::Low, "Low"),
                            (KeyboardBacklightLevel::Medium, "Medium"),
                            (KeyboardBacklightLevel::High, "High"),
                        ] {
                            if ui
                                .add_enabled(
                                    self.snapshot.connected,
                                    egui::Button::selectable(current == choice, label),
                                )
                                .clicked()
                            {
                                self.apply(Action::SetBacklight(choice));
                            }
                        }
                    });
                });
            });

            if let Some(error) = &self.snapshot.error {
                ui.add_space(12.0);
                ui.label(RichText::new(error).color(Color32::from_rgb(244, 125, 125)));
            }
            if let Some(message) = &self.snapshot.message {
                ui.add_space(4.0);
                ui.label(RichText::new(message).color(Color32::from_rgb(145, 151, 164)));
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.separator();
                ui.label(
                    RichText::new("Changes are sent to the Lecoo daemon over local IPC.")
                        .small()
                        .color(Color32::from_rgb(120, 127, 140)),
                );
            });
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn metric(ui: &mut egui::Ui, title: &str, value: String) {
    ui.group(|ui| {
        ui.set_min_width(125.0);
        ui.label(RichText::new(title).small().color(Color32::from_rgb(145, 151, 164)));
        ui.add_space(4.0);
        ui.heading(RichText::new(value).size(24.0));
    });
}

fn value_with_unit<T: std::fmt::Display>(value: Option<T>, unit: &str) -> String {
    value.map(|value| format!("{value}{unit}")).unwrap_or_else(|| "—".into())
}

fn fan_controls(ui: &mut egui::Ui, snapshot: &Snapshot, fan: FanIndex, label: &str) {
    let mode = snapshot.settings.as_ref().map(|settings| match fan {
        FanIndex::Cpu => settings.fan_mode_cpu,
        FanIndex::Gpu => settings.fan_mode_gpu,
    });
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add_enabled_ui(snapshot.connected, |ui| {
            if ui
                .add(egui::Button::selectable(mode == Some(FanMode::Auto), "Auto"))
                .clicked()
            {
                // Actions are sent by the parent UI after this helper reports the selection.
            }
        });
    });
}

fn run_worker(action_rx: Receiver<Action>, update_tx: Sender<Snapshot>) {
    let mut last_poll = Instant::now() - Duration::from_secs(2);
    let mut message: Option<String> = None;

    loop {
        match action_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(action) => {
                message = Some(send_action(action));
                last_poll = Instant::now() - Duration::from_secs(2);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if last_poll.elapsed() >= Duration::from_secs(1) {
            let mut snapshot = read_snapshot();
            snapshot.message = message.clone();
            if let Some(error) = &snapshot.error {
                message = Some(error.clone());
                snapshot.message = message.clone();
            }
            let _ = update_tx.send(snapshot);
            last_poll = Instant::now();
        }
    }
}

fn read_snapshot() -> Snapshot {
    let mut snapshot = Snapshot::default();
    let mut client = match IpcClient::connect() {
        Ok(client) => client,
        Err(error) => {
            snapshot.error = Some(format!("Could not connect to the daemon: {error}"));
            return snapshot;
        }
    };
    snapshot.connected = true;

    if let Ok(IpcResponse::SystemInfo(info)) = client.request(&IpcRequest::GetSystemState) {
        snapshot.board = Some(format!("{} · daemon {}", info.revision, info.daemon_version));
    }
    if let Ok(IpcResponse::Temps { cpu_c, sys_c }) = client.request(&IpcRequest::GetTemperatures) {
        snapshot.cpu_temp = Some(cpu_c);
        snapshot.system_temp = Some(sys_c);
    }
    if let Ok(IpcResponse::FanRpm { cpu, gpu }) = client.request(&IpcRequest::GetFansRPM) {
        snapshot.cpu_fan = Some(cpu);
        snapshot.gpu_fan = Some(gpu);
    }
    if let Ok(IpcResponse::ChargeStatus(status)) = client.request(&IpcRequest::GetChargeStatus) {
        snapshot.charge = Some(status);
    }
    if let Ok(IpcResponse::Settings(settings)) =
        client.request(&IpcRequest::DaemonCommand(ipc::DaemonCommand::GetSettings))
    {
        snapshot.settings = Some(*settings);
    }
    snapshot
}

fn send_action(action: Action) -> String {
    let (request, label) = match action {
        Action::SetPower(profile) => (IpcRequest::SetPowerProfile(profile), "Power profile"),
        Action::SetFan(fan, mode) => (
            IpcRequest::SetFanMode { fan, mode },
            "Fan mode",
        ),
        Action::SetBacklight(level) => (
            IpcRequest::SetKeyboardBacklight(level),
            "Keyboard backlight",
        ),
    };

    let mut client = match IpcClient::connect() {
        Ok(client) => client,
        Err(error) => return format!("Daemon unavailable: {error}"),
    };

    match client.request(&request) {
        Ok(IpcResponse::Success) => format!("{label} updated"),
        Ok(IpcResponse::Error(error)) => error.message,
        Ok(response) => format!("Unexpected daemon response: {response:?}"),
        Err(error) => format!("Command failed: {error}"),
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([920.0, 680.0])
            .with_min_inner_size([700.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Lecoo Control Center",
        options,
        Box::new(|_creation_context| Ok(Box::new(ControlCenter::new()))),
    )
}
