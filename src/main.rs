#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::{time::Duration, thread, fs::File, sync::{mpsc::{Receiver, Sender}, atomic::{AtomicBool, Ordering}, Arc}, collections::VecDeque};
use egui::ProgressBar;
use gif::{Encoder, Repeat, Frame};
use scrap::{Display, Capturer};
use tokio::runtime::Runtime;

fn main() {

    let rt = Runtime::new().expect("Unable to create Runtime");

    // Enter the runtime so that `tokio::spawn` is available immediately.
    let _enter = rt.enter();

    // Execute the runtime in its own thread.
    // The future doesn't have to do anything. In this example, it just sleeps forever.
    std::thread::spawn(move || {
        rt.block_on(async {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        })
    });

    let options = eframe::NativeOptions {
        initial_window_size: Some(egui::vec2(300.0, 200.0)),
        icon_data: Some(load_icon(".\\icon.png")),
        ..Default::default()
    };
    eframe::run_native(
        "Clipper",
        options,
        Box::new(|_cc| Box::new(Clipper::default())),
    );
}

fn load_icon(path: &str) -> eframe::IconData {
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::open(path)
            .expect("Failed to open icon path")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };

    eframe::IconData {
        rgba: icon_rgba,
        width: icon_width,
        height: icon_height,
    }
}

struct Clipper {
    async_to_ui: (Sender<State>, Receiver<State>),
    path: String,
    recording: Arc<AtomicBool>,
    buffer_length_seconds: usize,
    current: State,
}

#[derive(PartialEq, Clone)]
enum State {
    Idle, Countdown(u8), Recording, Converting(f32), Encoding(f32)
}

impl Default for Clipper {
    fn default() -> Self {
        Self {
            async_to_ui: std::sync::mpsc::channel(),
            path: "wow.gif".to_string(),
            recording: Arc::new(AtomicBool::new(false)),
            buffer_length_seconds: 5,
            current: State::Idle,
        }
    }
}

impl eframe::App for Clipper {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        egui::CentralPanel::default().show(ctx, |ui| {

            ui.add_space(20.0);

            egui::Grid::new("my_grid")
                .num_columns(2)
                .spacing([40.0, 20.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label("File/path:");
                    ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("File name/path"));
                    ui.end_row();
                    ui.label("Buffer (seconds):");
                    ui.add(egui::DragValue::new(&mut self.buffer_length_seconds).speed(1.0))
            });
            ui.vertical_centered(|ui| {
                ui.add_space(10.0);

                let state = self.async_to_ui.1.try_recv().unwrap_or(self.current.clone());
                
                match state {
                    State::Idle => {
                        if ui.button("Start").clicked() {
                            self.recording.store(true, Ordering::SeqCst);
                            self.run();
                        }
                    },
                    State::Countdown(v) => {
                        ui.label(format!("Recording in {}...", v));
                    },
                    State::Recording => {
                        ui.label("Recording...");
                        if ui.button("Stop").clicked() {
                            self.recording.store(false, Ordering::SeqCst);
                        }
                    },
                    State::Converting(v) => {
                        ui.label("Converting (this may take a while)...");
                        let progress_bar = ProgressBar::new(v).show_percentage().animate(true);
                        ui.add(progress_bar);
                    },
                    State::Encoding(v) => {
                        ui.label("Encoding (this may take a while)...");
                        let progress_bar = ProgressBar::new(v).show_percentage().animate(true);
                        ui.add(progress_bar);
                    },
                }
                self.current = state;
            });
        });
    }
}

impl Clipper {

    fn run(&mut self) {
        let path = self.path.clone();
        let buffer_length_seconds = self.buffer_length_seconds.clone();
        let send = self.async_to_ui.0.clone();

        let recording = Arc::clone(&self.recording);

        tokio::spawn(async move {
            let second = Duration::from_secs(1);
            let one_sixthyth = second / 60;

            for i in 0..3 {
                let _ = send.send(State::Countdown(3 - i));
                thread::sleep(second);
            }

            let frame_delay = Duration::from_millis(40);

            let display = Display::primary().expect("Couldn't find primary display.");
            let mut capturer = Capturer::new(display).expect("Couldn't begin capture.");
            let (w, h) = (capturer.width(), capturer.height());

            let _ = send.send(State::Recording);

            let mut frame_data: VecDeque<Vec<u8>> = VecDeque::new();
            while recording.load(Ordering::SeqCst) {
                let frame = match capturer.frame() {
                    Ok(f) => f,
                    Err(error) => {
                        if error.kind() == std::io::ErrorKind::WouldBlock {
                            thread::sleep(one_sixthyth);
                            continue;
                        } else {
                            panic!("Error: {}", error);
                        }
                    }
                };
                let data = frame.to_vec();
                frame_data.push_back(data);
                if frame_data.len() > buffer_length_seconds * 13 /* about 13 frames per second */ {
                    frame_data.pop_front();
                }
                thread::sleep(frame_delay);
            }

            let _ = send.send(State::Encoding(0.0));

            let frame_count = frame_data.len() as f32;
            let incr = frame_count / 100.0 / 100.0;
            let mut current = 0.0;

            let mut rgba_frame_data: Vec<Vec<u8>> = Vec::with_capacity(frame_data.len());
            for frame in frame_data {
                let mut new_frame: Vec<u8> = Vec::with_capacity(frame.len());
                let rows = frame.chunks(w * 4);
                for (i, row) in rows.into_iter().enumerate() {
                    if i % 2 == 0 {
                        continue;
                    }
                    let mut row = row.chunks(4).into_iter().enumerate()
                        .filter(|(byte_ind, _)| byte_ind % 2 == 0)
                        .map(|(_, byte)| {
                            vec![byte[2], byte[1], byte[0], byte[3]] // flip BGRA to RGBA
                        })
                        .flatten()
                        .collect::<Vec<u8>>();
                    new_frame.append(&mut row);
                }
                rgba_frame_data.push(new_frame);

                current += incr;
                let _ = send.send(State::Converting(current));
            }

            current = 0.0;
            let _ = send.send(State::Encoding(current));

            let width = (w / 2) as u16;
            let height = (h / 2) as u16;
            
            let frame_data = rgba_frame_data;

            let color_map = &[0xFF, 0xFF, 0xFF, 0, 0, 0];
            let mut image = File::create(path.as_str()).expect("Could not create file");
            let mut encoder = Encoder::new(&mut image, width, height, color_map).expect("Could not create encoder");
            encoder.set_repeat(Repeat::Infinite).expect("Could not set encoder property");
            for mut frame_data_single in frame_data {
                let mut frame = Frame::from_rgba_speed(width, height, &mut frame_data_single, 30);
                frame.delay = 7;
                frame.make_lzw_pre_encoded();
                encoder.write_lzw_pre_encoded_frame(&frame).expect("Could not write frame to encoder");

                current += incr;
                let _ = send.send(State::Encoding(current));
            }

            let _ = send.send(State::Idle);
        });
    }
}