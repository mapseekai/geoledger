//! Reproduce a macOS leaks false positive with an owned, unaligned buffer view.
use bytes::{Buf, BytesMut};
use std::{io, sync::Mutex};

static BUFFER: Mutex<Option<BytesMut>> = Mutex::new(None);
const CAPACITY: usize = 96 * 1024;
const CONSUMED: usize = 31_265;

fn pause(message: &str) -> io::Result<()> {
    println!(
        "pid={} {message}; press Enter to continue",
        std::process::id()
    );
    let mut line = String::new();
    assert!(io::stdin().read_line(&mut line)? > 0);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // End the allocating thread so stale stack pointers cannot hide the signal.
    *BUFFER.lock()? = Some(
        std::thread::spawn(|| {
            let mut buffer = BytesMut::with_capacity(CAPACITY);
            buffer.resize(CONSUMED, 0xa5);
            buffer.advance(CONSUMED);
            assert!(buffer.is_empty());
            buffer
        })
        .join()
        .map_err(|_| io::Error::other("allocating thread panicked"))?,
    );
    pause("owned buffer consumed")?;
    {
        let mut guard = BUFFER.lock()?;
        let buffer = guard.as_mut().ok_or("buffer missing")?;
        assert!(buffer.try_reclaim(CAPACITY));
        assert!(buffer.capacity() >= CAPACITY);
        buffer.extend_from_slice(b"x");
        assert_eq!(buffer.as_ref(), b"x");
    }
    pause("same buffer reclaimed and usable")?;
    drop(BUFFER.lock()?.take());
    pause("buffer dropped")?;
    Ok(())
}
