//! `catalog.*` (upstream's `ControlDispatcher`): a served catalog ([`crate::catalog`]) answers them, and an app with
//! windows refuses them as upstream's `ControlActionsDefaults` does, since only a headless server owns a catalog and a
//! remote window must never bind to a live app's windows. `catalog.op` checks its arguments first either way. The
//! strings name this build's server (conformance/docs/rust-build.md > Other strings of the build's own).

use ngterm_proto::command::CatalogCmd;
use ngterm_proto::{CatalogReceipt, Request, Response};

use super::args_of;
use crate::Core;
use crate::catalog::operations::CatalogPosition;
use crate::error::ControlError;
use crate::model::naming::trimmed_or_none;

/// Answers `cmd`: the served catalog's subscription (its stream starts once this reply is out) or operation lookup,
/// else the refusal.
pub(crate) fn dispatch(core: &mut Core, cmd: CatalogCmd, request: &Request) -> Result<Response, ControlError> {
    if !core.serves_catalog() {
        return Err(refusal(cmd, request));
    }
    match cmd {
        CatalogCmd::Subscribe => Ok(Response::success()),
        CatalogCmd::Op => {
            check_op_args(request).map_err(ControlError::rejected)?;
            let op_id = request.target.as_deref().and_then(trimmed_or_none).unwrap_or_default();
            let args = args_of(request);
            let sent = args
                .epoch
                .as_deref()
                .and_then(trimmed_or_none)
                .zip(args.rev)
                .map(|(epoch, rev)| CatalogPosition { epoch, rev });
            Ok(core.catalog_operation(&op_id, sent.as_ref()))
        }
    }
}

const SUBSCRIBE_SERVER_ONLY: &str = "catalog.subscribe is served only by ngterm-headless";
const OP_SERVER_ONLY: &str = "catalog.op is served only by ngterm-headless";

/// Why this host does not answer `cmd`.
pub(crate) fn refusal(cmd: CatalogCmd, request: &Request) -> ControlError {
    match cmd {
        CatalogCmd::Subscribe => ControlError::rejected(SUBSCRIBE_SERVER_ONLY),
        CatalogCmd::Op => ControlError::rejected(check_op_args(request).err().unwrap_or(OP_SERVER_ONLY)),
    }
}

/// `catalog.op`'s arguments, checked before the host answers or refuses it.
fn check_op_args(request: &Request) -> Result<(), &'static str> {
    let op_id = request.target.as_deref().and_then(trimmed_or_none);
    if !op_id.is_some_and(|op_id| CatalogReceipt::is_valid_op_id(&op_id)) {
        return Err("catalog.op requires an operation id");
    }
    let args = args_of(request);
    let epoch = args.epoch.as_deref().and_then(trimmed_or_none);
    if epoch.is_some() != args.rev.is_some() {
        return Err("catalog.op takes epoch and rev together");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ngterm_proto::{Args, Command};

    use super::*;

    fn operation(target: Option<&str>, epoch: Option<&str>, rev: Option<i64>) -> String {
        let request = Request {
            cmd: Command::Catalog(CatalogCmd::Op),
            target: target.map(str::to_owned),
            args: (epoch.is_some() || rev.is_some()).then(|| Args {
                epoch: epoch.map(str::to_owned),
                rev,
                ..Args::default()
            }),
        };
        refusal(CatalogCmd::Op, &request).to_string()
    }

    /// Upstream's `aMacAppRefusesCatalogSubscribe`.
    #[test]
    fn subscribe_is_the_servers_alone() {
        let request = Request::new(Command::Catalog(CatalogCmd::Subscribe));
        assert_eq!(
            refusal(CatalogCmd::Subscribe, &request).to_string(),
            "catalog.subscribe is served only by ngterm-headless"
        );
    }

    /// Upstream's `aMacAppRefusesCatalogOp` and the dispatcher's argument checks ahead of it.
    #[test]
    fn op_checks_its_arguments_then_is_the_servers_alone() {
        let needs_id = "catalog.op requires an operation id";
        let long = "x".repeat(129);
        for target in [
            None,
            Some(""),
            Some("  "),
            Some("a b"),
            Some("a/b"),
            Some(long.as_str()),
        ] {
            assert_eq!(operation(target, None, None), needs_id, "{target:?}");
        }
        let together = "catalog.op takes epoch and rev together";
        assert_eq!(operation(Some("op-1"), Some("E"), None), together);
        assert_eq!(operation(Some("op-1"), None, Some(3)), together);
        // a blank epoch is no epoch
        assert_eq!(operation(Some("op-1"), Some("  "), Some(3)), together);
        let server_only = "catalog.op is served only by ngterm-headless";
        assert_eq!(operation(Some("op-1"), None, None), server_only);
        assert_eq!(operation(Some(" a.b_c:d-1 "), Some("E"), Some(3)), server_only);
    }
}
