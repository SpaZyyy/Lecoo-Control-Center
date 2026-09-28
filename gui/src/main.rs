#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText};
use lecoo_types::ec_types::{FanIndex, FanMode, KeyboardBacklightLevel, PowerProfile};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

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
    charge_mode: Option<String>,
    battery_percent: Option<u8>,
    power_profile: Option<PowerProfile>,
    keyboard_backlight: Option<KeyboardBacklightLevel>,
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
            .name("lecoo-gui-cli".into())
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
            ui.label(self.snapshot.board.as_deref().unwrap_or("Hardware status and controls"));
            ui.add_space(14.0);

            ui.columns(4, |columns| {
                metric(&mut columns[0], "CPU TEMPERATURE", value_with_unit(self.snapshot.cpu_temp, "°C"));
                metric(
                    &mut columns[1],
                    "SYSTEM TEMPERATURE",
                    value_with_unit(self.snapshot.system_temp, "°C"),
                );
                metric(&mut columns[2], "CPU FAN", value_with_unit(self.snapshot.cpu_fan, " RPM"));
                metric(&mut columns[3], "GPU FAN", value_with_unit(self.snapshot.gpu_fan, " RPM"));
            });

            ui.add_space(18.0);
            ui.columns(2, |columns| {
                columns[0].group(|ui| {
                    ui.set_min_height(182.0);
                    ui.heading("Performance");
                    ui.label("Power profile");
                    let profile = self.snapshot.power_profile;
                    ui.horizontal_wrapped(|ui| {
                        for (choice, label) in [
                            (PowerProfile::Silent, "Quiet"),
                            (PowerProfile::Default, "Balanced"),
                            (PowerProfile::Performance, "Performance"),
                        ] {
                            if ui
                                .add_enabled(
                                    self.snapshot.connected,
                                    egui::Button::new(label).selected(profile == Some(choice)),
                                )
                                .clicked()
                            {
                                self.apply(Action::SetPower(choice));
                            }
                        }
                    });

                    ui.add_space(10.0);
                    ui.label("Fans");
                    fan_controls(ui, self.snapshot.connected, &self.actions, FanIndex::Cpu, "CPU");
                    fan_controls(ui, self.snapshot.connected, &self.actions, FanIndex::Gpu, "GPU");
                });

                columns[1].group(|ui| {
                    ui.set_min_height(182.0);
                    ui.heading("Battery & lighting");
                    match self.snapshot.battery_percent {
                        Some(percent) => {
                            ui.horizontal(|ui| {
                                ui.label("Battery");
                                ui.label(RichText::new(format!("{percent}%")).strong());
                            });
                            if let Some(mode) = &self.snapshot.charge_mode {
                                ui.label(format!("Charge mode  {mode}"));
                            }
                        }
                        None => {
                            ui.label("Battery status unavailable");
                        }
                    }

                    ui.add_space(12.0);
                    ui.label("Keyboard backlight");
                    let current = self.snapshot.keyboard_backlight;
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
                                    egui::Button::new(label).selected(current == Some(choice)),
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
                    RichText::new("Reads and changes use the installed lecoo-ctrl utility.")
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

fn fan_controls(ui: &mut egui::Ui, connected: bool, actions: &Sender<Action>, fan: FanIndex, label: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add_enabled_ui(connected, |ui| {
            if ui.button("Auto").clicked() {
                let _ = actions.send(Action::SetFan(fan, FanMode::Auto));
            }
            if ui.button("Full").clicked() {
                let _ = actions.send(Action::SetFan(fan, FanMode::Full));
            }
        });
    });
}

fn run_worker(action_rx: Receiver<Action>, update_tx: Sender<Snapshot>) {
    let mut last_poll = Instant::now() - Duration::from_secs(2);
    let mut last_details = Instant::now() - Duration::from_secs(10);
    let mut message: Option<String> = None;
    let mut snapshot = Snapshot::default();

    loop {
        match action_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(action) => {
                message = Some(send_action(action));
                last_poll = Instant::now() - Duration::from_secs(2);
                last_details = Instant::now() - Duration::from_secs(10);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if last_poll.elapsed() >= Duration::from_secs(1) {
            match read_telemetry(&mut snapshot) {
                Ok(()) => {
                    snapshot.connected = true;
                    snapshot.error = None;
                    if last_details.elapsed() >= Duration::from_secs(5) {
                        read_details(&mut snapshot);
                        last_details = Instant::now();
                    }
                }
                Err(error) => {
                    snapshot.connected = false;
                    snapshot.error = Some(error);
                    snapshot.cpu_temp = None;
                    snapshot.system_temp = None;
                    snapshot.cpu_fan = None;
                    snapshot.gpu_fan = None;
                }
            }
            snapshot.message = message.clone();
            if update_tx.send(snapshot.clone()).is_err() {
                break;
            }
            last_poll = Instant::now();
        }
    }
}

fn run_cli(args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("lecoo-ctrl.exe");
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let output = command.output().map_err(|error| format!("Cannot start lecoo-ctrl: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() {
            format!("lecoo-ctrl {} failed ({})", args.join(" "), output.status)
        } else {
            error
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_telemetry(snapshot: &mut Snapshot) -> Result<(), String> {
    let temps = run_cli(&["temps"])?;
    let fans = run_cli(&["fans"])?;
    let (cpu_temp, system_temp) = two_values(&temps).ok_or("Cannot read temperatures")?;
    let (cpu_fan, gpu_fan) = two_values(&fans).ok_or("Cannot read fan speeds")?;
    snapshot.cpu_temp = u8::try_from(cpu_temp).ok();
    snapshot.system_temp = u8::try_from(system_temp).ok();
    snapshot.cpu_fan = Some(cpu_fan);
    snapshot.gpu_fan = Some(gpu_fan);
    Ok(())
}

fn read_details(snapshot: &mut Snapshot) {
    if let Ok(info) = run_cli(&["info"]) {
        let lines: Vec<_> = info.lines().collect();
        if let (Some(chip), Some(version)) = (lines.first(), lines.last()) {
            snapshot.board = Some(format!("{} · {}", chip.trim(), version.trim()));
        }
    }
    if let Ok(charge) = run_cli(&["charge"]) {
        snapshot.charge_mode = charge.lines().nth(1).and_then(after_colon);
        snapshot.battery_percent =
            charge.lines().nth(2).and_then(number_after_colon).and_then(|n| u8::try_from(n).ok());
    }
    if let Ok(power) = run_cli(&["power"]) {
        snapshot.power_profile = power.lines().last().and_then(after_colon).and_then(|value| {
            match value.to_ascii_lowercase().as_str() {
                "silent" => Some(PowerProfile::Silent),
                "default" => Some(PowerProfile::Default),
                "performance" | "perf" => Some(PowerProfile::Performance),
                _ => None,
            }
        });
    }
    if let Ok(kbd) = run_cli(&["kbd"]) {
        snapshot.keyboard_backlight = kbd.lines().last().and_then(after_colon).and_then(|value| match value
            .to_ascii_lowercase()
            .as_str()
        {
            "off" => Some(KeyboardBacklightLevel::Off),
            "low" => Some(KeyboardBacklightLevel::Low),
            "medium" => Some(KeyboardBacklightLevel::Medium),
            "high" => Some(KeyboardBacklightLevel::High),
            _ => None,
        });
    }
}

fn after_colon(line: &str) -> Option<String> {
    line.split_once(':').map(|(_, value)| value.trim().to_owned())
}

fn number_after_colon(line: &str) -> Option<u16> {
    let value = line.split_once(':')?.1;
    let number: String = value
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    number.parse().ok()
}

fn two_values(output: &str) -> Option<(u16, u16)> {
    let mut values = output.lines().filter_map(number_after_colon);
    Some((values.next()?, values.next()?))
}

fn send_action(action: Action) -> String {
    let (args, label): (Vec<&str>, &str) = match action {
        Action::SetPower(PowerProfile::Silent) => (vec!["power", "silent"], "Power profile"),
        Action::SetPower(PowerProfile::Default) => (vec!["power", "default"], "Power profile"),
        Action::SetPower(PowerProfile::Performance) => (vec!["power", "perf"], "Power profile"),
        Action::SetFan(FanIndex::Cpu, FanMode::Auto) => (vec!["fan", "cpu", "auto"], "CPU fan"),
        Action::SetFan(FanIndex::Cpu, FanMode::Full) => (vec!["fan", "cpu", "full"], "CPU fan"),
        Action::SetFan(FanIndex::Gpu, FanMode::Auto) => (vec!["fan", "gpu", "auto"], "GPU fan"),
        Action::SetFan(FanIndex::Gpu, FanMode::Full) => (vec!["fan", "gpu", "full"], "GPU fan"),
        Action::SetFan(_, _) => return "Unsupported fan mode blocked".into(),
        Action::SetBacklight(KeyboardBacklightLevel::Off) => (vec!["kbd", "off"], "Keyboard backlight"),
        Action::SetBacklight(KeyboardBacklightLevel::Low) => (vec!["kbd", "low"], "Keyboard backlight"),
        Action::SetBacklight(KeyboardBacklightLevel::Medium) => (vec!["kbd", "medium"], "Keyboard backlight"),
        Action::SetBacklight(KeyboardBacklightLevel::High) => (vec!["kbd", "high"], "Keyboard backlight"),
        Action::SetBacklight(_) => return "Unsupported backlight level blocked".into(),
    };

    match run_cli(&args) {
        Ok(_) => format!("{label} updated"),
        Err(error) => format!("{label}: {error}"),
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
