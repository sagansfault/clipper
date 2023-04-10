use crossbeam_channel::{unbounded, Receiver, Sender};
use device_query::{DeviceQuery, DeviceState, Keycode};
use gif::{Encoder, Frame, Repeat};
use scrap::{Capturer, Display};
use std::{
    collections::VecDeque,
    fs::File,
    thread,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

#[derive(PartialEq, Clone)]
enum State {
    Idle,
    Recording,
    Encoding(u8),
}

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

    let mut state: State = State::Idle;
    let path = "wow.gif".to_string();
    let clip_length = 5;
    let fps = 30;

    let state_signal: (Sender<State>, Receiver<State>) = unbounded();
    let data_receiver: (Sender<Vec<u8>>, Receiver<Vec<u8>>) = unbounded();

    let display = Display::primary().expect("Couldn't find primary display.");
    let (width, height) = (display.width(), display.height());

    let mut buf: VecDeque<Vec<u8>> = VecDeque::new();
    println!("Press \"Alt+A\" to start/clip recording");

    let mut previous: Vec<Keycode> = vec![];
    loop {
        if let Ok(data) = data_receiver.1.try_recv() {
            buf.push_back(data);
            if buf.len() > clip_length * fps as usize {
                buf.pop_front();
            }
        }
        let keyboard = DeviceState::new().get_keys();
        if check_keys(&keyboard) && !check_keys(&previous) {
            match &state {
                State::Idle => {
                    let signal_receiver = state_signal.1.clone();
                    let data_sender = data_receiver.0.clone();
                    tokio::spawn(async move {
                        run(fps, signal_receiver, data_sender).await;
                    });
                    state = State::Recording;
                    let _ = state_signal.0.send(state.clone());
                    println!("Recording...");
                }
                State::Recording => {
                    println!("Encoding...");
                    state = State::Encoding(0);
                    let _ = state_signal.0.send(state.clone());
                    let cloned = buf.clone();
                    buf.clear(); // clear right after cloning to have the state be as clean as possible
                    encode(cloned, path.clone(), width, height, fps);
                    println!("Done!");
                    state = State::Idle;
                }
                State::Encoding(_) => {}
            }
        }
        previous = keyboard;
    }
}

fn check_keys(keys: &Vec<Keycode>) -> bool {
    keys.contains(&Keycode::LAlt) && keys.contains(&Keycode::A)
}

async fn run(fps: u8, signal_receiver: Receiver<State>, data_sender: Sender<Vec<u8>>) {
    // have to remake thi
    let display = Display::primary().expect("Couldn't find primary display.");
    let mut capturer = Capturer::new(display).expect("Couldn't begin capture.");

    let ms_per_frame = Duration::from_millis((1000.0 / fps as f64) as u64);
    let one_sixthyth = Duration::from_secs(1) / 60;

    let mut instant = Instant::now();
    loop {
        if let Ok(state) = signal_receiver.try_recv() {
            if state != State::Recording {
                break;
            }
        }
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
        let elapsed = instant.elapsed();
        if ms_per_frame > elapsed {
            spin_sleep::sleep(ms_per_frame - elapsed);
        };
        instant = Instant::now();
        let _ = data_sender.send(data);
    }
}

fn encode(buf: VecDeque<Vec<u8>>, path: String, width: usize, height: usize, fps: u8) {
    // flip BGRA to RGBA
    let buf = buf
        .iter()
        .map(|frame| {
            frame
                .chunks(4)
                .into_iter()
                .map(|byte| vec![byte[2], byte[1], byte[0], byte[3]])
                .flatten()
                .collect::<Vec<u8>>()
        })
        .collect::<Vec<Vec<u8>>>();

    let mut frame_data = vec![];
    for frame in buf {
        let mut new_frame: Vec<u8> = Vec::with_capacity(frame.len());
        let rows = frame.chunks(width * 4);
        for (i, row) in rows.into_iter().enumerate() {
            if i % 2 == 0 {
                continue;
            }
            let mut row = row
                .chunks(4)
                .into_iter()
                .enumerate()
                .filter(|(byte_ind, _)| byte_ind % 2 == 0)
                .map(|(_, val)| val.to_vec())
                .flatten()
                .collect::<Vec<u8>>();
            new_frame.append(&mut row);
        }
        frame_data.push(new_frame);
    }

    let half_width = (width / 2) as u16;
    let half_height = (height / 2) as u16;

    let color_map = &[0xFF, 0xFF, 0xFF, 0, 0, 0];
    let mut image = File::create(path.as_str()).expect("Could not create file");
    let mut encoder = Encoder::new(&mut image, half_width, half_height, color_map)
        .expect("Could not create encoder");
    encoder
        .set_repeat(Repeat::Infinite)
        .expect("Could not set encoder property");
    for mut frame_data_single in frame_data {
        let mut frame = Frame::from_rgba_speed(half_width, half_height, &mut frame_data_single, 30);
        frame.delay = (100.0 / fps as f64) as u16;

        frame.make_lzw_pre_encoded();
        encoder
            .write_lzw_pre_encoded_frame(&frame)
            .expect("Could not write frame to encoder");
    }
}
