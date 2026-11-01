//! `session.tint` and `workspace.tint`, and the creation tint `session.new` and `workspace.new` take. Validation and
//! its messages follow `dispatchTint`, `tintUpdate` and `creationTint` in upstream's
//! `ControlDispatcher+SessionMetadata.swift`; the effects follow `agterm/Control/ControlServer+Tint.swift`.
//!
//! Both commands share one parse, checked before the target resolves, so a refused call never touches the model. A
//! clear refuses a color or placement rather than ignoring it, so a contradictory raw request never half-applies.
//! Upstream's default `ControlActions` overloads that refuse a tint on a host predating tints have no counterpart:
//! one dispatcher serves the app and `ngterm-headless`.

use ngterm_proto::{Args, ControlResult};

use super::{Outcome, session_store, with_id, workspace_store};
use crate::Core;
use crate::error::ControlError;
use crate::model::{SidebarTint, TintColor, TintPart, TintParts, TintUpdate};

/// `session.tint`: the session's tint set (a set without `tintOn` keeps the row's own parts) or cleared. An unchanged
/// value neither saves nor emits.
pub(crate) fn session(core: &mut Core, target: Option<&str>, args: &Args) -> Outcome {
    let update = tint_update(args, "session.tint")?;
    let (store, id) = session_store(core, target, args.window.as_deref())?;
    let current = store.session(id).and_then(|session| session.tint);
    store.set_session_tint(update.resolved(current), id);
    Ok(with_id(id))
}

/// `workspace.tint`: the workspace row's own tint; its sessions keep theirs.
pub(crate) fn workspace(core: &mut Core, target: Option<&str>, args: &Args) -> Result<ControlResult, ControlError> {
    let update = tint_update(args, "workspace.tint")?;
    let (store, id) = workspace_store(core, target, args.window.as_deref())?;
    let current = store.workspace(id).and_then(|workspace| workspace.tint);
    store.set_workspace_tint(update.resolved(current), id);
    Ok(with_id(id))
}

/// A set or clear: the mode is required, a set needs a color, a clear takes neither a color nor a placement.
pub(crate) fn tint_update(args: &Args, command: &str) -> Result<TintUpdate, ControlError> {
    match args.mode.as_deref().unwrap_or_default() {
        "set" => {
            let raw = args
                .tint
                .as_deref()
                .ok_or_else(|| ControlError::rejected(format!("{command} set requires a color")))?;
            Ok(TintUpdate::Set(tint_color(raw)?, tint_parts(args.tint_on.as_deref())?))
        }
        "clear" if args.tint.is_some() || args.tint_on.is_some() => Err(ControlError::rejected(format!(
            "{command} clear takes no color or placement"
        ))),
        "clear" => Ok(TintUpdate::Clear),
        mode => Err(ControlError::rejected(format!("invalid tint mode: {mode} (set|clear)"))),
    }
}

/// `session.new`/`workspace.new`'s optional tint, parsed before anything is created so an invalid one leaves nothing
/// behind; a placement without a color is refused.
pub(crate) fn creation_tint(args: &Args) -> Result<Option<SidebarTint>, ControlError> {
    let Some(raw) = args.tint.as_deref() else {
        return match args.tint_on {
            Some(_) => Err(ControlError::rejected("--tint-on requires --tint")),
            None => Ok(None),
        };
    };
    let color = tint_color(raw)?;
    let parts = tint_parts(args.tint_on.as_deref())?;
    Ok(Some(SidebarTint::new(color, parts.unwrap_or(TintParts::ICON))))
}

/// A color, matched case-insensitively.
fn tint_color(raw: &str) -> Result<TintColor, ControlError> {
    TintColor::from_wire(&raw.to_lowercase())
        .ok_or_else(|| ControlError::rejected(format!("invalid tint: {raw} ({})", TintColor::valid_names_list())))
}

/// An optional placement list; an empty one or an unknown part is refused, naming the list as sent.
fn tint_parts(raw: Option<&[String]>) -> Result<Option<TintParts>, ControlError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    TintParts::parse(raw).map(Some).ok_or_else(|| {
        ControlError::rejected(format!(
            "invalid tint placement: {} ({})",
            raw.join(","),
            TintPart::valid_names_list()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(edit: impl FnOnce(&mut Args)) -> Args {
        let mut args = Args::default();
        edit(&mut args);
        args
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn rejected(message: &str) -> ControlError {
        ControlError::rejected(message)
    }

    /// `sessionTintSetRoutesColorAndParsedParts`, `workspaceTintRoutesToTheWorkspaceAction`: the color matched
    /// case-insensitively, the parts parsed into a set, a clear carrying nothing.
    #[test]
    fn a_set_parses_its_color_and_parts_and_a_clear_takes_nothing() {
        let set = |tint: &str, on: Option<Vec<String>>| {
            tint_update(
                &args(|a| {
                    a.mode = Some("set".to_owned());
                    a.tint = Some(tint.to_owned());
                    a.tint_on = on;
                }),
                "session.tint",
            )
        };
        assert_eq!(set("Red", None), Ok(TintUpdate::Set(TintColor::Red, None)));
        assert_eq!(
            set("blue", Some(strings(&["name", "icon"]))),
            Ok(TintUpdate::Set(TintColor::Blue, Some(TintParts::BOTH)))
        );
        assert_eq!(
            set("gray", Some(strings(&["name"]))),
            Ok(TintUpdate::Set(TintColor::Gray, Some(TintParts::NAME)))
        );
        assert_eq!(
            tint_update(&args(|a| a.mode = Some("clear".to_owned())), "workspace.tint"),
            Ok(TintUpdate::Clear)
        );
    }

    /// `sessionTintRefusesContradictoryOrUnknownInput`.
    #[test]
    fn contradictory_or_unknown_input_is_refused_with_upstreams_words() {
        let cases = [
            (
                args(|a| a.mode = Some("set".to_owned())),
                "session.tint set requires a color",
            ),
            (
                args(|a| {
                    a.mode = Some("set".to_owned());
                    a.tint = Some("teal".to_owned());
                }),
                "invalid tint: teal (red|orange|yellow|green|blue|purple|gray)",
            ),
            (
                args(|a| {
                    a.mode = Some("set".to_owned());
                    a.tint = Some("red".to_owned());
                    a.tint_on = Some(strings(&["strip"]));
                }),
                "invalid tint placement: strip (icon,name)",
            ),
            (
                args(|a| {
                    a.mode = Some("set".to_owned());
                    a.tint = Some("red".to_owned());
                    a.tint_on = Some(strings(&[]));
                }),
                "invalid tint placement:  (icon,name)",
            ),
            (
                args(|a| {
                    a.mode = Some("clear".to_owned());
                    a.tint = Some("red".to_owned());
                }),
                "session.tint clear takes no color or placement",
            ),
            (
                args(|a| {
                    a.mode = Some("clear".to_owned());
                    a.tint_on = Some(strings(&["icon"]));
                }),
                "session.tint clear takes no color or placement",
            ),
            (
                args(|a| a.tint = Some("red".to_owned())),
                "invalid tint mode:  (set|clear)",
            ),
        ];
        for (args, message) in cases {
            assert_eq!(tint_update(&args, "session.tint"), Err(rejected(message)), "{args:?}");
        }
    }

    /// `creationRefusesABadTintBeforeCreatingAnything`, and the accepted forms: no tint, a color on the icon by
    /// default, a color on the listed parts.
    #[test]
    fn a_creation_tint_defaults_to_the_icon_and_refuses_a_bad_one() {
        assert_eq!(creation_tint(&Args::default()), Ok(None));
        assert_eq!(
            creation_tint(&args(|a| a.tint = Some("Orange".to_owned()))),
            Ok(Some(SidebarTint::on_icon(TintColor::Orange)))
        );
        assert_eq!(
            creation_tint(&args(|a| {
                a.tint = Some("red".to_owned());
                a.tint_on = Some(strings(&["name"]));
            })),
            Ok(Some(SidebarTint::new(TintColor::Red, TintParts::NAME)))
        );
        let cases = [
            (
                args(|a| a.tint_on = Some(strings(&["icon"]))),
                "--tint-on requires --tint",
            ),
            (
                args(|a| a.tint = Some("teal".to_owned())),
                "invalid tint: teal (red|orange|yellow|green|blue|purple|gray)",
            ),
            (
                args(|a| {
                    a.tint = Some("red".to_owned());
                    a.tint_on = Some(strings(&["both"]));
                }),
                "invalid tint placement: both (icon,name)",
            ),
        ];
        for (args, message) in cases {
            assert_eq!(creation_tint(&args), Err(rejected(message)), "{args:?}");
        }
    }
}
