//! The host side of the pixel file for the probes: what a program that shows the guest's frames does
//! with `Event::FrameChannel`, `Event::Frame`, session loss and a gone reader (rewrite-bridge 0.4's
//! "Migrating pixel consumers"). Included by the examples with `#[path]`.
#![allow(dead_code)]

use std::path::Path;

use rewrite_bridge::link::{FrameChannel, FrameData, FrameMiss, FrameReader, FrameReady};

/// One reader and the image it last presented.
///
/// `rgba` and `depth` hold the shown image only while [`FrameSink::shown`] is `Some`; a read goes into
/// scratch buffers first, because a failed read may leave partial bytes behind.
#[derive(Default)]
pub struct FrameSink {
    reader: Option<FrameReader>,
    pub rgba: Vec<u8>,
    pub depth: Vec<f32>,
    scratch_rgba: Vec<u8>,
    scratch_depth: Vec<f32>,
    shown: Option<FrameData>,
    /// Announcements that gave no new image (torn, replaced, out of range).
    pub missed: u64,
}

impl FrameSink {
    /// Drops the reader and the shown image.
    pub fn clear(&mut self) {
        self.reader = None;
        self.rgba.clear();
        self.depth.clear();
        self.shown = None;
    }

    /// A new pixel file: the old reader and image go first, also when the new file cannot be opened.
    /// Returns whether the new file is open.
    pub fn channel(&mut self, c: &FrameChannel) -> bool {
        self.clear();
        match FrameReader::open(Path::new(&c.path)) {
            Ok(r) => {
                self.reader = Some(r);
                true
            }
            Err(_) => false,
        }
    }

    /// The session ended.
    pub fn lost(&mut self) {
        self.clear();
    }

    /// Presents an announced frame if it is newer than the last one shown and the copy is intact.
    /// A reader that reports `Gone` is dropped together with the image.
    pub fn present(&mut self, ready: &FrameReady) -> Option<FrameData> {
        let reader = self.reader.as_ref()?;
        match reader.read_latest(ready, &mut self.scratch_rgba, Some(&mut self.scratch_depth)) {
            Ok(data) => {
                std::mem::swap(&mut self.rgba, &mut self.scratch_rgba);
                std::mem::swap(&mut self.depth, &mut self.scratch_depth);
                self.shown = Some(data.clone());
                Some(data)
            }
            Err(FrameMiss::Gone) => {
                self.clear();
                None
            }
            Err(_) => {
                self.missed += 1;
                None
            }
        }
    }

    pub fn shown(&self) -> Option<&FrameData> {
        self.shown.as_ref()
    }

    pub fn is_open(&self) -> bool {
        self.reader.is_some()
    }

    /// Duplicates and older numbers the reader rejected.
    pub fn stale_reads(&self) -> u64 {
        self.reader.as_ref().map_or(0, |r| r.stale_reads())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rewrite_bridge::link::{FrameFileOptions, FrameWriter};

    const W: u32 = 4;
    const H: u32 = 2;

    fn pixels(tag: u8) -> Vec<u8> {
        vec![tag; (W * H * 4) as usize]
    }

    fn depths(tag: f32) -> Vec<f32> {
        vec![tag; (W * H) as usize]
    }

    fn writer(tag: &str) -> FrameWriter {
        FrameWriter::create_temp_with(tag, &FrameFileOptions::new(W, H, 3).with_depth()).unwrap()
    }

    fn write(w: &mut FrameWriter, tag: u8) -> FrameReady {
        w.write_depth(&pixels(tag), Some(&depths(tag as f32)), W, H, 0, tag as u64)
            .unwrap()
    }

    #[test]
    fn an_older_announcement_arriving_late_does_not_replace_the_image() {
        let mut w = writer("sink-order");
        let (one, two) = (write(&mut w, 1), write(&mut w, 2));
        let mut sink = FrameSink::default();
        assert!(sink.channel(w.channel()));
        assert_eq!(sink.present(&two).unwrap().frame, 2);
        // frame 1 is still in its slot and intact, so a raw read would take it
        assert!(sink.present(&one).is_none());
        assert_eq!(sink.shown().unwrap().frame, 2);
        assert_eq!(sink.rgba, pixels(2));
        assert_eq!(sink.depth, depths(2.0));
        assert_eq!(sink.stale_reads(), 1);
        // the same number again is a duplicate too
        assert!(sink.present(&two).is_none());
        assert_eq!(sink.stale_reads(), 2);
    }

    #[test]
    fn a_miss_leaves_the_shown_image_alone() {
        let mut w = writer("sink-miss");
        let one = write(&mut w, 1);
        let mut sink = FrameSink::default();
        assert!(sink.channel(w.channel()));
        sink.present(&one).unwrap();
        let outside = FrameReady {
            frame: 9,
            slot: 7,
            camera_seq: 0,
            width: W,
            height: H,
        };
        assert!(sink.present(&outside).is_none());
        assert_eq!(sink.missed, 1);
        assert_eq!(sink.shown().unwrap().frame, 1);
        assert_eq!(sink.rgba, pixels(1));
    }

    #[test]
    fn a_new_channel_clears_the_image_even_when_it_cannot_be_opened() {
        let mut w = writer("sink-channel");
        let one = write(&mut w, 1);
        let mut sink = FrameSink::default();
        assert!(sink.channel(w.channel()));
        sink.present(&one).unwrap();
        let missing = FrameChannel {
            path: std::env::temp_dir()
                .join("frame-sink-no-such-file.frames")
                .to_string_lossy()
                .into_owned(),
            width: W,
            height: H,
            slots: 3,
        };
        assert!(!sink.channel(&missing));
        assert!(!sink.is_open());
        assert!(sink.shown().is_none() && sink.rgba.is_empty() && sink.depth.is_empty());
        assert!(sink.present(&one).is_none());
    }

    #[test]
    fn a_new_channel_restarts_the_numbers() {
        let mut a = writer("sink-first");
        let mut b = writer("sink-second");
        let a5 = write(&mut a, 5);
        let b1 = write(&mut b, 1);
        let mut sink = FrameSink::default();
        assert!(sink.channel(a.channel()));
        sink.present(&a5).unwrap();
        assert!(sink.channel(b.channel()));
        // frame 1 of the new file is shown although 5 was shown from the old one
        assert_eq!(sink.present(&b1).unwrap().frame, 1);
        assert_eq!(sink.rgba, pixels(1));
    }

    #[test]
    fn session_loss_clears_the_image_and_the_reader() {
        let mut w = writer("sink-lost");
        let one = write(&mut w, 1);
        let mut sink = FrameSink::default();
        assert!(sink.channel(w.channel()));
        sink.present(&one).unwrap();
        sink.lost();
        assert!(!sink.is_open());
        assert!(sink.shown().is_none() && sink.rgba.is_empty());
        assert!(sink.present(&one).is_none());
    }

    #[test]
    fn a_gone_reader_is_dropped_with_the_image() {
        let mut w = writer("sink-gone");
        let (one, two) = (write(&mut w, 1), write(&mut w, 2));
        let mut sink = FrameSink::default();
        assert!(sink.channel(w.channel()));
        sink.present(&one).unwrap();
        // the file shrinks under the reader
        std::fs::OpenOptions::new()
            .write(true)
            .open(w.path())
            .unwrap()
            .set_len(64)
            .unwrap();
        assert!(sink.present(&two).is_none());
        assert!(!sink.is_open());
        assert!(sink.shown().is_none() && sink.rgba.is_empty() && sink.depth.is_empty());
    }
}
