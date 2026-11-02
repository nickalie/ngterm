use futures_lite::future::block_on;
use ngterm_term::GridSize;

use super::fake::{FakeTransport, Script};
use super::{Channel, ChannelEnd, ChannelInput, ChannelOutput, Connection, PEER_GONE, Transport, TransportError};
use crate::api::Machine;

fn machine(host: &str) -> Machine {
    Machine {
        id: "m1".to_owned(),
        label: "box".to_owned(),
        host: host.to_owned(),
        port: 22,
        user: "nick".to_owned(),
        key_id: "key".to_owned(),
        pinned_host_key: None,
    }
}

const SIZE: GridSize = GridSize {
    cols: 80,
    rows: 24,
    cell_width: 9,
    cell_height: 18,
};

#[test]
fn output_arrives_in_order_then_the_end_repeats() {
    let (mut channel, peer) = Channel::pair();
    block_on(async {
        assert!(peer.data(b"one".to_vec()).await);
        assert!(peer.stderr(b"two".to_vec()).await);
        peer.end(ChannelEnd::Exit {
            code: Some(3),
            signal: None,
        })
        .await;
    });

    let end = ChannelOutput::End(ChannelEnd::Exit {
        code: Some(3),
        signal: None,
    });
    assert_eq!(block_on(channel.reader.read()), ChannelOutput::Data(b"one".to_vec()));
    assert_eq!(channel.reader.blocking_read(), ChannelOutput::Stderr(b"two".to_vec()));
    assert_eq!(block_on(channel.reader.read()), end);
    assert_eq!(block_on(channel.reader.read()), end);
}

#[test]
fn a_peer_gone_without_an_end_reads_as_lost() {
    let (mut channel, peer) = Channel::pair();
    drop(peer);

    assert_eq!(
        channel.reader.blocking_read(),
        ChannelOutput::End(ChannelEnd::Lost(PEER_GONE.to_owned()))
    );
    assert!(!channel.writer.write(b"x"), "nobody takes input any more");
}

#[test]
fn input_reaches_the_peer_in_order_from_every_writer() {
    let (channel, mut peer) = Channel::pair();
    let other = channel.writer.clone();

    assert!(channel.writer.write(b"ls\r"));
    assert!(other.resize(SIZE));
    assert!(channel.writer.eof());
    assert!(other.close());

    assert_eq!(peer.try_input(), Some(ChannelInput::Data(b"ls\r".to_vec())));
    assert_eq!(peer.try_input(), Some(ChannelInput::Resize(SIZE)));
    assert_eq!(peer.try_input(), Some(ChannelInput::Eof));
    assert_eq!(peer.try_input(), Some(ChannelInput::Close));
    assert_eq!(peer.try_input(), None);
    drop(channel);
    drop(other);
    assert_eq!(block_on(peer.input()), None);
}

#[test]
fn a_dropped_reader_closes_the_peer() {
    let (channel, peer) = Channel::pair();
    assert!(!peer.is_closed());
    drop(channel.reader);

    assert!(peer.is_closed());
    assert!(!block_on(peer.data(b"lost".to_vec())));
}

#[test]
fn the_fake_refuses_a_host_and_accepts_it_again() {
    let transport = FakeTransport::new();
    transport.refuse("box", "no route to host");

    let refused = block_on(transport.connect(&machine("box"))).err();
    assert_eq!(
        refused,
        Some(TransportError::Unreachable("no route to host".to_owned()))
    );

    transport.accept("box");
    assert!(block_on(transport.connect(&machine("box"))).is_ok());
    assert_eq!(transport.connects(), ["m1", "m1"]);
}

#[test]
fn a_scripted_exec_answers_and_ends() {
    let transport = FakeTransport::new();
    transport.script("ngtermctl live tree", Script::ok("tree"));
    transport.script("missing", Script::failing(127, "not found"));
    let connection = block_on(transport.connect(&machine("box"))).expect("connects");

    let mut tree = block_on(connection.exec("ngtermctl live tree", None)).expect("opens");
    let mut missing = block_on(connection.exec("missing", None)).expect("opens");

    assert_eq!(tree.reader.blocking_read(), ChannelOutput::Data(b"tree".to_vec()));
    assert_eq!(
        tree.reader.blocking_read(),
        ChannelOutput::End(ChannelEnd::Exit {
            code: Some(0),
            signal: None
        })
    );
    assert_eq!(
        missing.reader.blocking_read(),
        ChannelOutput::Stderr(b"not found".to_vec())
    );
    assert_eq!(
        missing.reader.blocking_read(),
        ChannelOutput::End(ChannelEnd::Exit {
            code: Some(127),
            signal: None
        })
    );
    assert!(
        transport.take_opened().is_empty(),
        "scripted channels are not handed out"
    );
}

#[test]
fn an_unscripted_exec_is_handed_to_the_test_with_its_pty() {
    let transport = FakeTransport::new();
    let connection = block_on(transport.connect(&machine("box"))).expect("connects");

    let mut channel = block_on(connection.exec("ngterm-session attach", Some(SIZE))).expect("opens");
    let mut opened = transport.take_opened();

    assert_eq!(opened.len(), 1);
    let opened = opened.remove(0);
    assert_eq!(
        (opened.machine_id.as_str(), opened.command.as_str(), opened.pty),
        ("m1", "ngterm-session attach", Some(SIZE))
    );
    assert!(channel.writer.write(b"x"));
    let mut peer = opened.peer;
    assert_eq!(peer.try_input(), Some(ChannelInput::Data(b"x".to_vec())));
    block_on(peer.end(ChannelEnd::Lost("reset".to_owned())));
    assert_eq!(
        channel.reader.blocking_read(),
        ChannelOutput::End(ChannelEnd::Lost("reset".to_owned()))
    );
    assert_eq!(transport.execs().len(), 1);
}

#[test]
fn a_closed_connection_refuses_execs() {
    let transport = FakeTransport::new();
    let connection = block_on(transport.connect(&machine("box"))).expect("connects");

    connection.close();
    connection.close();

    assert_eq!(
        block_on(connection.exec("ngtermctl live tree", None)).err(),
        Some(TransportError::Closed)
    );
    assert_eq!(transport.closed(), ["m1"], "closed once");
}

#[test]
fn closing_a_connection_ends_its_open_channels_as_lost() {
    let transport = FakeTransport::new();
    let connection = block_on(transport.connect(&machine("box"))).expect("connects");
    let mut channel = block_on(connection.exec("ngterm-session attach", None)).expect("opens");
    let opened = transport.take_opened();

    connection.close();

    assert_eq!(
        channel.reader.blocking_read(),
        ChannelOutput::End(ChannelEnd::Lost(TransportError::Closed.to_string()))
    );
    assert!(opened[0].peer.is_closed(), "the reader is done with it");
}

#[test]
fn a_held_connect_finishes_once_released() {
    let transport = FakeTransport::new();
    transport.hold("box");
    let held = machine("box");
    let mut connect = Box::pin(transport.connect(&held));

    assert!(block_on(futures_lite::future::poll_once(&mut connect)).is_none());
    transport.release("box");

    assert!(block_on(connect).is_ok());
    assert_eq!(transport.connects(), ["m1"]);
}
