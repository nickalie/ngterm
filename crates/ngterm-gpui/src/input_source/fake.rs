//! The unit tests' fake system behind the performer (the lock's tests here, the page's in `crate::html_overlay`):
//! the layouts it offers, the one selected, and what was asked of it.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::TestAppContext;

use super::{Backend, Failure, KeyboardLayout, install};

pub(crate) const US: &str = "English (US)";
pub(crate) const RU: &str = "Russian";

/// What the fake system holds and what was asked of it.
#[derive(Default)]
pub(crate) struct System {
    pub(crate) enabled: Vec<String>,
    pub(crate) current: String,
    pub(crate) selected: Vec<String>,
    pub(crate) refuse: bool,
    pub(crate) probes: usize,
    pub(crate) reads: usize,
}

struct Fake(Rc<RefCell<System>>);

impl Backend for Fake {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        let mut system = self.0.borrow_mut();
        system.reads += 1;
        Ok(system.enabled.iter().cloned().map(KeyboardLayout::named).collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        Ok(self.0.borrow().current.clone())
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let mut system = self.0.borrow_mut();
        if system.refuse {
            return Err("refused".into());
        }
        system.selected.push(id.to_owned());
        system.current = id.to_owned();
        Ok(())
    }
}

/// The performer over a fake system on `current`, with US and Russian enabled.
pub(crate) fn installed(cx: &mut TestAppContext, current: &str) -> Rc<RefCell<System>> {
    let system = Rc::new(RefCell::new(System {
        enabled: vec![US.to_owned(), RU.to_owned()],
        current: current.to_owned(),
        ..System::default()
    }));
    let probed = Rc::clone(&system);
    cx.update(|cx| {
        install(
            Box::new(move || {
                probed.borrow_mut().probes += 1;
                Ok(Box::new(Fake(probed)) as Box<dyn Backend>)
            }),
            None,
            cx,
        );
    });
    system
}
