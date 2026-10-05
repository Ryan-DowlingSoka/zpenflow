//! One Penflow GUI per user session.
//!
//! Two GUIs means two services racing for the same `localabstract:penflow`
//! reverse rule, each removing the other's when its session ends, so the
//! tablet ends up with nothing to connect to. A second launch therefore
//! asks the running GUI to show its window and exits.
//!
//! A named mutex marks the running instance. The relaunch-as-admin flow
//! starts the elevated copy while the old one is still exiting, so a new
//! process waits a few seconds for the mutex before concluding another GUI
//! is really running.

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        INFINITE,
    };

    const MUTEX_NAME: &str = "Local\\PenflowGui.Instance";
    const SHOW_EVENT_NAME: &str = "Local\\PenflowGui.Show";
    /// How long a new process waits for a previous instance to exit (the
    /// admin relaunch hands over within ~100 ms).
    const HANDOVER_WAIT: Duration = Duration::from_secs(5);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Held for the life of the process; the OS releases it on exit.
    pub struct InstanceGuard {
        _mutex: HANDLE,
    }

    // SAFETY: the handle is only kept alive, never used from two threads.
    unsafe impl Send for InstanceGuard {}
    unsafe impl Sync for InstanceGuard {}

    /// Become the running instance, or return None after asking the
    /// running one to show its window.
    pub fn acquire() -> Option<InstanceGuard> {
        acquire_named(MUTEX_NAME, SHOW_EVENT_NAME, HANDOVER_WAIT)
    }

    pub(super) fn acquire_named(
        mutex_name: &str,
        show_event: &str,
        wait: Duration,
    ) -> Option<InstanceGuard> {
        let name = wide(mutex_name);
        let mutex = match unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) } {
            Ok(h) => h,
            // Can't create or open it (e.g. owned by an elevated instance
            // with a stricter DACL): another GUI is running.
            Err(_) => {
                signal_show(show_event);
                return None;
            }
        };
        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            return Some(InstanceGuard { _mutex: mutex });
        }
        // Someone else owns it; give a handing-over instance time to exit.
        let r = unsafe { WaitForSingleObject(mutex, wait.as_millis() as u32) };
        if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
            return Some(InstanceGuard { _mutex: mutex });
        }
        let _ = unsafe { CloseHandle(mutex) };
        signal_show(show_event);
        None
    }

    fn signal_show(event_name: &str) {
        let name = wide(event_name);
        if let Ok(event) = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) } {
            let _ = unsafe { SetEvent(event) };
            let _ = unsafe { CloseHandle(event) };
        }
    }

    /// In the running instance: call `on_show` whenever a later launch asks
    /// for the window.
    pub fn listen_for_show(on_show: impl Fn() + Send + 'static) {
        let name = wide(SHOW_EVENT_NAME);
        // Auto-reset event: each signal wakes the waiter once.
        let Ok(event) = (unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())) }) else {
            return;
        };
        let raw = event.0 as isize;
        std::thread::spawn(move || {
            let event = HANDLE(raw as *mut _);
            loop {
                if unsafe { WaitForSingleObject(event, INFINITE) } != WAIT_OBJECT_0 {
                    break;
                }
                on_show();
            }
        });
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct InstanceGuard;
    pub fn acquire() -> Option<InstanceGuard> {
        Some(InstanceGuard)
    }
    pub fn listen_for_show(_on_show: impl Fn() + Send + 'static) {}
}

pub use imp::*;

#[cfg(all(test, windows))]
mod tests {
    use super::imp::*;
    use std::sync::mpsc;
    use std::time::Duration;

    // Mutex ownership is per thread, so threads stand in for processes.
    fn names(tag: &str) -> (String, String) {
        let id = std::process::id();
        (
            format!(r"Local\PenflowTest.{tag}.{id}.Mutex"),
            format!(r"Local\PenflowTest.{tag}.{id}.Show"),
        )
    }

    #[test]
    fn a_second_instance_is_refused_while_the_first_runs() {
        let (mutex, event) = names("refuse");
        let (held_tx, held_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let (m, e) = (mutex.clone(), event.clone());
        let first = std::thread::spawn(move || {
            let guard = acquire_named(&m, &e, Duration::from_millis(100));
            held_tx.send(guard.is_some()).unwrap();
            done_rx.recv().unwrap(); // keep running until told to stop
        });
        assert!(held_rx.recv().unwrap(), "first instance should start");
        assert!(acquire_named(&mutex, &event, Duration::from_millis(200)).is_none());
        done_tx.send(()).unwrap();
        first.join().unwrap();
    }

    #[test]
    fn a_new_instance_takes_over_when_the_old_one_exits_within_the_wait() {
        let (mutex, event) = names("handover");
        let (held_tx, held_rx) = mpsc::channel();
        let (m, e) = (mutex.clone(), event.clone());
        let old = std::thread::spawn(move || {
            let _guard = acquire_named(&m, &e, Duration::from_millis(100));
            held_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(300)); // exiting, like the admin relaunch
        });
        held_rx.recv().unwrap();
        // Waits out the old instance instead of giving up immediately.
        assert!(acquire_named(&mutex, &event, Duration::from_secs(5)).is_some());
        old.join().unwrap();
    }
}
