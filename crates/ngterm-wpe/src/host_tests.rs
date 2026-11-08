use super::*;

fn local() -> (Host, Receiver<Command>) {
    let (commands, queue) = async_channel::unbounded();
    let (stop, stopped) = std::sync::mpsc::channel();
    drop(stop);
    (
        Host {
            engine: Engine::Local(Local {
                commands,
                stopped: std::sync::Mutex::new(Some(stopped)),
            }),
        },
        queue,
    )
}

#[test]
fn local_commands_preserve_history_and_scripts() {
    let (host, commands) = local();
    let view = ViewId(9);
    assert_eq!(host.backend(), Backend::InProcess);
    assert_eq!(host.worker_pid(), None);
    host.stop(view);
    host.go_forward(view);
    host.set_scripts(
        view,
        vec![UserScript {
            source: "window.x = 1".to_owned(),
            time: crate::InjectionTime::End,
            world: Some("isolated".to_owned()),
        }],
    );
    assert!(matches!(commands.recv_blocking().unwrap(), Command::View(id, ViewCommand::Stop) if id == view));
    assert!(matches!(commands.recv_blocking().unwrap(), Command::View(id, ViewCommand::GoForward) if id == view));
    let Command::View(id, ViewCommand::Scripts(scripts)) = commands.recv_blocking().unwrap() else {
        panic!("scripts")
    };
    assert_eq!(id, view);
    assert_eq!(scripts[0].source, "window.x = 1");
    assert_eq!(scripts[0].world.as_deref(), Some("isolated"));
}

#[test]
fn local_commands_preserve_background_and_focus() {
    let (host, commands) = local();
    let view = ViewId(9);
    host.set_background(view, Some(Rgba::TRANSPARENT));
    host.set_focused(view, false);
    assert!(
        matches!(commands.recv_blocking().unwrap(), Command::View(id, ViewCommand::Background(Some(color))) if id == view && color == Rgba::TRANSPARENT)
    );
    assert!(matches!(commands.recv_blocking().unwrap(), Command::View(id, ViewCommand::Focused(false)) if id == view));
}

#[test]
fn local_evaluation_preserves_its_world_and_answers_through_the_returned_channel() {
    let (host, commands) = local();
    let result = host.evaluate(ViewId(7), "2 + 2".to_owned(), Some("reader".to_owned()));
    let Command::View(id, ViewCommand::Evaluate { script, world, reply }) = commands.recv_blocking().unwrap() else {
        panic!("evaluation")
    };
    assert_eq!(
        (id, script.as_str(), world.as_deref()),
        (ViewId(7), "2 + 2", Some("reader"))
    );
    reply.try_send(Ok(Some("4".to_owned()))).unwrap();
    assert_eq!(result.recv_blocking().unwrap(), Ok(Some("4".to_owned())));
    drop(commands);
}

#[test]
fn a_local_stop_posts_quit_once_even_when_the_engine_already_ended() {
    let (host, commands) = local();
    host.shutdown();
    assert!(matches!(commands.recv_blocking().unwrap(), Command::Quit));
    host.shutdown();
    drop(host);
    assert!(commands.recv_blocking().is_err());
}

#[test]
fn invalid_configuration_is_rejected_before_starting() {
    let invalid = HostConfig {
        frames: crate::FrameMode::Shm,
        schemes: vec!["1bad".to_owned()],
    };
    assert!(matches!(Host::start_worker(invalid), Err(StartError::Config(_))));
}
