use std::io::Read;
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use mio_signals::{Signal, SignalSet, Signals, send_signal};

#[test]
fn signal_bit_or() {
    // `Signal` and `Signal` (and `Signal`).
    assert_eq!(
        Signal::Terminate | Signal::Quit | Signal::Interrupt | Signal::User1 | Signal::User2,
        SignalSet::all()
    );
    // `Signal` and `SignalSet`.
    assert_eq!(
        Signal::Terminate | SignalSet::from(Signal::Quit),
        Signal::Terminate | Signal::Quit
    );

    // `SignalSet` and `Signal`.
    assert_eq!(
        SignalSet::from(Signal::Interrupt) | Signal::Quit,
        Signal::Quit | Signal::Interrupt
    );
    // `SignalSet` and `SignalSet`.
    assert_eq!(
        SignalSet::from(Signal::Interrupt) | SignalSet::from(Signal::Terminate),
        Signal::Interrupt | Signal::Terminate
    );

    // Overwriting.
    let signal = Signal::Terminate; // This is done to avoid a clippy warning.
    assert_eq!(signal | Signal::Terminate, Signal::Terminate.into());
    assert_eq!(Signal::Terminate | SignalSet::all(), SignalSet::all());
    assert_eq!(SignalSet::all() | Signal::Quit, SignalSet::all());
    assert_eq!(SignalSet::all() | SignalSet::all(), SignalSet::all());
}

#[test]
fn signal_set() {
    let tests = vec![
        (
            SignalSet::all(),
            5,
            vec![
                Signal::Interrupt,
                Signal::Terminate,
                Signal::Quit,
                Signal::User1,
                Signal::User2,
            ],
            "Interrupt|Quit|Terminate|User1|User2",
        ),
        (
            Signal::Interrupt.into(),
            1,
            vec![Signal::Interrupt],
            "Interrupt",
        ),
        (
            Signal::Terminate.into(),
            1,
            vec![Signal::Terminate],
            "Terminate",
        ),
        (Signal::Quit.into(), 1, vec![Signal::Quit], "Quit"),
        (
            Signal::Interrupt | Signal::Terminate,
            2,
            vec![Signal::Interrupt, Signal::Terminate],
            "Interrupt|Terminate",
        ),
        (
            Signal::Interrupt | Signal::Quit,
            2,
            vec![Signal::Interrupt, Signal::Quit],
            "Interrupt|Quit",
        ),
        (
            Signal::Terminate | Signal::Quit,
            2,
            vec![Signal::Terminate, Signal::Quit],
            "Quit|Terminate",
        ),
        (
            Signal::Interrupt | Signal::Terminate | Signal::Quit,
            3,
            vec![Signal::Interrupt, Signal::Terminate, Signal::Quit],
            "Interrupt|Quit|Terminate",
        ),
        (
            Signal::Interrupt | Signal::User1 | Signal::User2,
            3,
            vec![Signal::Interrupt, Signal::User1, Signal::User1],
            "Interrupt|User1|User2",
        ),
    ];

    for (set, size, expected, expected_fmt) in tests {
        let set: SignalSet = set;
        assert_eq!(set.len(), size);

        // Test `contains`.
        let mut contains_iter = (&expected).iter().cloned();
        while let Some(signal) = contains_iter.next() {
            assert!(set.contains(signal));
            assert!(set.contains::<SignalSet>(signal.into()));

            // Set of the remaining signals.
            let mut contains_set: SignalSet = signal.into();
            for signal in contains_iter.clone() {
                contains_set = contains_set | signal;
            }
            assert!(set.contains(contains_set));
        }

        // Test `SignalSetIter`.
        assert_eq!(set.into_iter().len(), size);
        assert_eq!(set.into_iter().count(), size);
        assert_eq!(set.into_iter().size_hint(), (size, Some(size)));
        let signals: Vec<Signal> = set.into_iter().collect();
        assert_eq!(signals.len(), expected.len());
        for expected in expected {
            assert!(signals.contains(&expected));
        }

        let got_fmt = format!("{:?}", set);
        let got_iter_fmt = format!("{:?}", set.into_iter());
        assert_eq!(got_fmt, expected_fmt);
        assert_eq!(got_iter_fmt, expected_fmt);
    }
}

#[test]
fn signal_set_iter_length() {
    let set = Signal::Interrupt | Signal::Terminate | Signal::Quit | Signal::User1 | Signal::User2;
    let mut iter = set.into_iter();

    assert!(iter.next().is_some());
    assert_eq!(iter.len(), 4);
    assert_eq!(iter.size_hint(), (4, Some(4)));

    assert!(iter.next().is_some());
    assert_eq!(iter.len(), 3);
    assert_eq!(iter.size_hint(), (3, Some(3)));

    assert!(iter.next().is_some());
    assert_eq!(iter.len(), 2);
    assert_eq!(iter.size_hint(), (2, Some(2)));

    assert!(iter.next().is_some());
    assert_eq!(iter.len(), 1);
    assert_eq!(iter.size_hint(), (1, Some(1)));

    assert!(iter.next().is_some());
    assert_eq!(iter.len(), 0);
    assert_eq!(iter.size_hint(), (0, Some(0)));

    assert!(iter.next().is_none());
}

#[test]
fn receive_no_signal() {
    let mut signals = Signals::new(SignalSet::all()).expect("unable to create Signals");
    assert_eq!(signals.receive().expect("unable to receive signal"), None);
}

#[test]
fn example() {
    let child = run_example("signal_handling");

    // Give the process some time to startup.
    sleep(Duration::from_millis(200));

    let pid = child.id() as u32;

    send_signal(pid, Signal::User1).unwrap();
    send_signal(pid, Signal::User2).unwrap();
    send_signal(pid, Signal::Interrupt).unwrap();
    send_signal(pid, Signal::Quit).unwrap();
    send_signal(pid, Signal::Terminate).unwrap();

    let output = read_output(child);
    // On Linux the signals seem to be delivered out of order, which is
    // perfectly fine, but does change the output.
    // In the end we do get all signals, which is what we want.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let want = format!(
        "Call `kill -s TERM {}` to stop the process\nGot interrupt signal\nGot quit signal\nGot user signal 1\nGot user signal 2\nGot terminate signal\n",
        pid
    );
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let want = format!(
        "Call `kill -s TERM {}` to stop the process\nGot user signal 1\nGot user signal 2\nGot interrupt signal\nGot quit signal\nGot terminate signal\n",
        pid
    );
    assert_eq!(output, want);
}

/// Wrapper around a `command::Child` that kills the process when dropped, even
/// if the test failed. Sometimes the child command would survive the test when
/// running then in a loop (e.g. with `cargo watch`). This caused problems when
/// trying to bind to the same port again.
struct ChildCommand {
    inner: Child,
}

impl Deref for ChildCommand {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.inner
    }
}

impl DerefMut for ChildCommand {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.inner
    }
}

impl Drop for ChildCommand {
    fn drop(&mut self) {
        let _ = self.inner.kill();
        self.inner.wait().expect("can't wait on child process");
    }
}

/// Run an example, not waiting for it to complete, but it does wait for it to
/// be build.
fn run_example(name: &'static str) -> ChildCommand {
    Command::new(format!("target/debug/examples/{}", name))
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map(|inner| ChildCommand { inner })
        .expect("unable to run example")
}

/// Read the standard output of the child command.
fn read_output(mut child: ChildCommand) -> String {
    child.wait().expect("error running example");

    let mut stdout = child.stdout.take().unwrap();
    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .expect("error reading output of example");
    output
}
