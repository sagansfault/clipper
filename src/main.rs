use std::{time::{Duration, Instant}, thread, fs::File, sync::mpsc::{Receiver, Sender}};

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
        ..Default::default()
    };
    eframe::run_native(
        "Clipper",
        options,
        Box::new(|_cc| Box::new(Clipper::default())),
    );
}

struct Clipper {
    send: Sender<State>,
    recv: Receiver<State>,
    path: String,
    duration: u8,
    current: State,
}

#[derive(PartialEq, Clone)]
enum State {
    Idle, Countdown(u8), Recording, Encoding
}

impl Default for Clipper {
    fn default() -> Self {
        let (send, recv) = std::sync::mpsc::channel();
        Self { 
            send,
            recv,
            path: "wow.gif".to_string(),
            duration: 5,
            current: State::Idle,
        }
    }
}

impl eframe::App for Clipper {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {

            egui::Grid::new("my_grid")
                .num_columns(2)
                .spacing([40.0, 20.0])
                .striped(true)
                .show(ui, |ui| {

                    ui.label("File/path:");
                    ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("File name/path"));
                    ui.end_row();

                    ui.label("Duration:");
                    ui.add(egui::DragValue::new(&mut self.duration).clamp_range(3..=20));
                    ui.end_row();
            });
            ui.vertical_centered(|ui| {
                ui.add_space(10.0);

                let state = self.recv.try_recv().unwrap_or(self.current.clone());
                
                match state {
                    State::Idle => {
                        if ui.button("Clip").clicked() {
                            assert!(self.path.ends_with(".gif"), "Path must end with .gif");
                            self.run();
                        }
                    },
                    State::Countdown(v) => {
                        ui.label(format!("Recording in {}...", v));
                    },
                    State::Recording => {
                        ui.label("Recording...");
                    },
                    State::Encoding => {
                        ui.label("Encoding (this may take a while)...");
                    },
                }
                self.current = state;
            });
        });
    }
}

impl Clipper {

    fn run(&mut self) {
        let duration = self.duration.clone();
        let path = self.path.clone();
        let send = self.send.clone();

        tokio::spawn(async move {
            let second = Duration::from_secs(1);
            let one_sixthyth = second / 60;

            for i in 0..3 {
                let _ = send.send(State::Countdown(3 - i));
                thread::sleep(second);
            }

            let frame_delay = Duration::from_millis(20);

            let display = Display::primary().expect("Couldn't find primary display.");
            let mut capturer = Capturer::new(display).expect("Couldn't begin capture.");
            let (w, h) = (capturer.width(), capturer.height());

            let _ = send.send(State::Recording);

            let duration = Duration::from_secs(duration as u64);
            let elapsed = Instant::now();
            let mut frame_data: Vec<Vec<u8>> = vec![];
            while elapsed.elapsed() <= duration {
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
                frame_data.push(data);
                thread::sleep(frame_delay);
            }

            let mut rgba_frame_data: Vec<Vec<u8>> = Vec::with_capacity(frame_data.len());
            for frame in frame_data {
                let mut new_frame: Vec<u8> = Vec::with_capacity(frame.len());
                for byte in frame.chunks(4) {
                    new_frame.push(byte[2]); // R
                    new_frame.push(byte[1]); // G
                    new_frame.push(byte[0]); // B
                    new_frame.push(byte[3]); // A
                }
                rgba_frame_data.push(new_frame);
            }
            
            let frame_data = rgba_frame_data;

            let width = w as u16;
            let height = h as u16;

            let _ = send.send(State::Encoding);

            let color_map = &[0xFF, 0xFF, 0xFF, 0, 0, 0];
            let mut image = File::create(path.as_str()).expect("Could not create file");
            let mut encoder = Encoder::new(&mut image, width, height, color_map).expect("Could not create encoder");
            encoder.set_repeat(Repeat::Infinite).expect("Could not set encoder property");
            for mut frame_data_single in frame_data {
                let mut frame = Frame::from_rgba_speed(width, height, &mut frame_data_single, 30);
                frame.delay = 5;
                frame.make_lzw_pre_encoded();
                //encoder.write_frame(&frame)?;
                encoder.write_lzw_pre_encoded_frame(&frame).expect("Could not write frame to encoder");
            }

            let _ = send.send(State::Idle);
        });
    }
}