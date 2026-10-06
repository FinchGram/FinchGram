//! The media viewer (the design's media-viewer.js): the open chat's photos, videos and GIFs over the
//! whole window, one at a time. A photo is shown in its largest size, downloaded when it is shown,
//! so that it can be zoomed into. A video is downloaded whole, then plays through mpv
//! (src/player/); a GIF loops. "Download" copies the file into the Downloads folder; "locate"
//! closes the viewer and brings its message into sight.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{Local, TimeZone};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{File, MessageContent};
use super::store::{self, Names};
use super::{conversation, files, with_ui};
use crate::player;
use crate::{Conversation, MainWindow, Viewer};

/// How long "Saved to Downloads" stays.
const SAVED_SHOWN: Duration = Duration::from_millis(2200);
/// What the user asked for is downloaded before anything else.
const ASKED: i32 = 32;
/// The speeds the viewer steps through (Viewer.speed): the design's.
const SPEEDS: [f64; 4] = [1.0, 1.5, 2.0, 0.5];

pub fn connect(ui: &MainWindow) {
    let viewer = ui.global::<Viewer>();
    viewer.on_view(|id, autoplay| {
        if let Ok(id) = id.parse() {
            view(id, autoplay);
        }
    });
    viewer.on_close(close);
    viewer.on_show(show);
    viewer.on_play_pause(play_pause);
    viewer.on_seek(seek);
    viewer.on_next_speed(|| {
        let mut speed = 0;
        with_viewer(|viewer| {
            speed = (viewer.get_speed() + 1) % 4;
            viewer.set_speed(speed);
        });
        player::set_speed(SPEEDS[speed as usize]);
    });
    viewer.on_toggle_mute(|| {
        let mut muted = false;
        with_viewer(|viewer| {
            muted = !viewer.get_muted();
            viewer.set_muted(muted);
        });
        player::set_muted(muted);
    });
    viewer.on_download(download);
    viewer.on_locate(locate);
}

/// Open the viewer at a message's photo or video; a video starts playing when `autoplay`.
fn view(message_id: i64, autoplay: bool) {
    with_ui(|ui| {
        let names = Names::from(ui);
        let items = store::with(|store| store.viewer_items(&names)).unwrap_or_default();
        let id = SharedString::from(message_id.to_string());
        let Some(index) = items.iter().position(|item| item.id == id) else { return };
        let video = items[index].video;
        let viewer = ui.global::<Viewer>();
        viewer.set_items(ModelRc::new(VecModel::from(items)));
        viewer.set_index(index as i32);
        viewer.set_zoom(0);
        viewer.set_position(0.0);
        viewer.set_speed(0);
        viewer.set_muted(false);
        viewer.set_saved(SharedString::new());
        viewer.set_playing(false);
        viewer.set_loading(false);
        viewer.set_ended(false);
        viewer.set_open(true);
        if autoplay && video {
            start_video(message_id);
        }
    });
    fetch_original(message_id);
}

/// Download the video shown, whole, then play it: unless something else is shown by then.
fn start_video(message_id: i64) {
    let Some((file, looping)) = message_content(message_id, |content| {
        Some((store::original(content)?.clone(), matches!(content, MessageContent::Animation { .. })))
    }) else {
        return;
    };
    with_viewer(|viewer| viewer.set_loading(true));
    files::download(&file, ASKED, move |path| {
        let mut still_shown = false;
        let (mut speed, mut muted) = (0, false);
        with_viewer(|viewer| {
            viewer.set_loading(false);
            still_shown = viewer.get_open() && shown_id().as_deref() == Some(message_id.to_string().as_str());
            speed = viewer.get_speed();
            muted = viewer.get_muted();
        });
        if still_shown {
            player::play(&path, looping, SPEEDS[speed as usize], muted);
        }
    });
}

fn play_pause() {
    let (mut playing, mut loading) = (false, false);
    with_viewer(|viewer| {
        playing = viewer.get_playing();
        loading = viewer.get_loading();
    });
    if loading {
        return;
    }
    if player::is_loaded() {
        player::set_paused(playing);
    } else if let Some(message_id) = shown_id().and_then(|id| id.parse().ok()) {
        start_video(message_id);
    }
}

fn seek(part: f32) {
    let mut duration = 0;
    with_viewer(|viewer| {
        let index = usize::try_from(viewer.get_index()).ok();
        duration = index.and_then(|index| slint::Model::row_data(&viewer.get_items(), index)).map_or(0, |item| item.duration);
    });
    if player::is_loaded() && duration > 0 {
        player::seek(f64::from(part) * f64::from(duration));
    }
}

/// Another item: a photo shown fitted, a video from its start (playing if one was).
fn show(index: i32) {
    let mut shown = None;
    let mut keep_playing = false;
    with_viewer(|viewer| {
        let items = viewer.get_items();
        let Some(item) = usize::try_from(index).ok().and_then(|index| slint::Model::row_data(&items, index)) else { return };
        keep_playing = item.video && viewer.get_playing();
        viewer.set_index(index);
        viewer.set_zoom(0);
        viewer.set_position(0.0);
        viewer.set_loading(false);
        shown = item.id.parse().ok();
    });
    player::stop();
    if let Some(message_id) = shown {
        if keep_playing {
            start_video(message_id);
        }
        fetch_original(message_id);
    }
}

fn close() {
    player::stop();
    with_viewer(|viewer| {
        viewer.set_open(false);
        viewer.set_playing(false);
        viewer.set_loading(false);
        viewer.set_items(ModelRc::default());
    });
}

/// A photo in its largest size, for zooming in; a video's still (Terminal shows no tiles that
/// would have fetched it), until it plays.
fn fetch_original(message_id: i64) {
    let file = message_content(message_id, |content| match content {
        MessageContent::Photo { .. } => store::original(content).cloned(),
        _ => store::picture(content)?.file.cloned(),
    });
    if let Some(file) = file {
        conversation::fetch_picture(file, ASKED);
    }
}

/// Close, and scroll the chat to the message of the item shown: to its album's row, which the
/// album's first message names.
fn locate() {
    let Some(message_id) = shown_id().and_then(|id| id.parse::<i64>().ok()) else { return };
    let Some((chat_id, row)) = store::with(|store| {
        let chat_id = store.open?;
        let messages = &store.histories.get(&chat_id)?.messages;
        let album = messages.get(&message_id)?.media_album_id;
        let first = (album != 0).then(|| messages.values().find(|message| message.media_album_id == album).map(|message| message.id)).flatten();
        Some((chat_id, first.unwrap_or(message_id)))
    })
    .flatten() else {
        return;
    };
    // The row may be above those shown: rows made just now are given a moment to be laid out.
    let extended = conversation::show_from(chat_id, row);
    close();
    let show = move || {
        with_ui(|ui| ui.global::<Conversation>().set_reveal(row.to_string().into()));
        // Cleared right after, so that locating the same message again scrolls again.
        slint::Timer::single_shot(Duration::from_millis(100), || {
            with_ui(|ui| ui.global::<Conversation>().set_reveal(SharedString::new()));
        });
    };
    if extended { slint::Timer::single_shot(Duration::from_millis(50), show) } else { show() }
}

/// The file of the item shown, whole, into the Downloads folder.
fn download() {
    let Some(message_id) = shown_id().and_then(|id| id.parse::<i64>().ok()) else { return };
    let Some((file, name)) = store::with(|store| {
        let message = store.histories.get(&store.open?)?.messages.get(&message_id)?;
        Some((store::original(&message.content)?.clone(), download_name(&message.content, message.date)))
    })
    .flatten() else {
        return;
    };
    download_file(&file, name);
}

fn download_file(file: &File, name: String) {
    files::download(file, ASKED, move |path| match save_to_downloads(Path::new(&path), &name) {
        Ok(saved) => {
            with_viewer(|viewer| viewer.set_saved(saved.into()));
            slint::Timer::single_shot(SAVED_SHOWN, || with_viewer(|viewer| viewer.set_saved(SharedString::new())));
        }
        Err(err) => eprintln!("viewer: cannot save {name} to Downloads: {err}"),
    });
}

/// A video's or a GIF's own file name; photos have none.
fn content_file_name(content: &MessageContent) -> Option<String> {
    let name = store::file_name(content);
    (!name.is_empty()).then(|| name.to_string())
}

/// What a message's photo, video or GIF is called in the Downloads folder: its own file name, or
/// for a photo when it was sent (`date`).
pub fn download_name(content: &MessageContent, date: i32) -> String {
    content_file_name(content).unwrap_or_else(|| photo_name(date))
}

/// A photo's name, from when it was sent, as Telegram's desktop app names them:
/// "photo_2026-09-30_14-19-05.jpg".
fn photo_name(unix: i32) -> String {
    match Local.timestamp_opt(i64::from(unix), 0).single() {
        Some(time) => time.format("photo_%Y-%m-%d_%H-%M-%S.jpg").to_string(),
        None => "photo.jpg".to_string(),
    }
}

/// Copy `from` into the Downloads folder as `name`, or as "name (2).ext" … when that is taken.
/// Returns the name it got.
pub fn save_to_downloads(from: &Path, name: &str) -> std::io::Result<String> {
    let folder = dirs::download_dir().ok_or_else(|| std::io::Error::other("no Downloads folder"))?;
    let target = free_path(&folder, name);
    std::fs::copy(from, &target)?;
    Ok(target.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())
}

fn free_path(folder: &Path, name: &str) -> PathBuf {
    // Only the last part of the name counts: it comes from other people's messages.
    let name = Path::new(name).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let first = folder.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| folder.join(format!("{stem} ({n}){extension}")))
        .find(|path| !path.exists())
        .unwrap_or(first)
}

fn shown_id() -> Option<String> {
    let mut id = None;
    with_viewer(|viewer| {
        let index = usize::try_from(viewer.get_index()).ok();
        id = index.and_then(|index| slint::Model::row_data(&viewer.get_items(), index)).map(|item| item.id.to_string());
    });
    id
}

fn message_content<R>(message_id: i64, read: impl FnOnce(&MessageContent) -> Option<R>) -> Option<R> {
    store::with(|store| {
        let chat_id = store.open?;
        let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
        read(&message.content)
    })
    .flatten()
}

fn with_viewer(change: impl FnOnce(&Viewer)) {
    with_ui(|ui| change(&ui.global::<Viewer>()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_gets_a_number_and_a_path_stays_in_the_folder() {
        let folder = std::env::temp_dir().join(format!("finchgram-downloads-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        assert_eq!(free_path(&folder, "clip.mp4"), folder.join("clip.mp4"));
        std::fs::write(folder.join("clip.mp4"), b"").expect("written");
        assert_eq!(free_path(&folder, "clip.mp4"), folder.join("clip (2).mp4"));
        assert_eq!(free_path(&folder, "../../etc/clip.mp4"), folder.join("clip (2).mp4"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn photos_are_named_by_when_they_were_sent() {
        let unix = Local.with_ymd_and_hms(2026, 9, 30, 14, 19, 5).single().expect("a time").timestamp() as i32;
        assert_eq!(photo_name(unix), "photo_2026-09-30_14-19-05.jpg");
    }
}
