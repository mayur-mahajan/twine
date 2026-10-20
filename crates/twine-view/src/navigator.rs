//! Typed screen navigation: [`navigator`], [`Navigator`], [`expect_navigator`] /
//! [`use_navigator`], leave guards
//! ([`Navigator::on_leave`], [`Leave`]) and the outcome of a request ([`NavOutcome`]).
//!
//! # Model
//!
//! A navigator holds a **stack of routes** — values of an application type `R` (usually an
//! enum, `Clone + PartialEq`) — and one **page function** `Fn(Scope, R) -> V` that turns a
//! route into the view of its page. Each page is an engine screen of its own with its own child
//! [`Scope`]; the screens are switched with the engine's screen-load animations
//! ([`Engine::load_screen_anim`], [`ScreenAnim`]).
//!
//! Requests ([`push`](Navigator::push), [`pop`](Navigator::pop),
//! [`replace`](Navigator::replace), [`pop_to`](Navigator::pop_to),
//! [`pop_to_root`](Navigator::pop_to_root), [`reset`](Navigator::reset)) change the route
//! stack **at once** (so [`stack`](Navigator::stack) and [`depth`](Navigator::depth) see them)
//! and are applied to the engine at the `Ui`'s next effect flush: several requests in one
//! handler become one transition, with the animation of the last one. Only the top page is
//! built: a page under it is built when it becomes the top again (pages restored with
//! [`NavigatorView::restore`] or pushed twice in one handler cost nothing until shown). A page
//! that leaves the stack is deleted — its scope disposed with every signal, effect, timer and
//! guard created inside — at the end of its animation (at once when it is not shown). When the
//! navigator itself goes away (its scope is disposed, e.g. with a [`when`](crate::when)
//! branch), its pages are deleted and the screen shown before it is shown again.
//!
//! A navigator owns the active screen of its display: use one per display. Navigation inside
//! a page (tabs, steps, panels) is a region of that page — [`when`](crate::when),
//! [`dynamic`](crate::dynamic), a `tabview` — not a nested navigator, which would load its
//! own screens over the page.
//!
//! # Memory
//!
//! The stack is two `Vec`s (routes, and per page an id and its screen) that grow only on
//! push and keep their capacity: about `size_of::<R>() + 32` bytes per page of the deepest
//! stack reached, plus the nodes and reactive state of the pages that are built (the top and
//! the pages below it that were shown before). Requests allocate nothing once the capacity is
//! reached ([`reset`](Navigator::reset) collects its routes into a `Vec`); building a page
//! allocates what its view needs, once. The page function and each guard are boxed once.
//!
//! The depth is bounded by [`Limits::max_nav_depth`](twine_core::Limits::max_nav_depth) (read
//! from the engine when the navigator is built): a request that would make the stack deeper
//! is refused with [`NavOutcome::Full`] and a
//! [`FaultKind::Capacity`](twine_core::fault::FaultKind::Capacity) fault (code
//! [`CapacityFault::NavDepth`](twine_core::fault::CapacityFault::NavDepth)) at the next effect
//! flush. With [`Limits::preallocate`](twine_core::Limits::preallocate) both stacks (and the
//! list of pages waiting to be deleted) are reserved to that depth when the navigator is
//! built, so requests and pops back to built pages allocate nothing after a seal (disposing a
//! page allocates nothing either); pushing a page that was never built builds its view, which
//! allocates.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

use twine_engine::{DisplayId, Engine, GroupId, NodeId, ScreenLoad, fmt_node_id};
use twine_reactive::{MissingContext, Scope, Signal, StoredValue};

use crate::access::EngineAccess;
use crate::build::{BuildCx, on_delete};
use crate::engine_queue::{EngineCmd, defer};
use crate::flow::{NAVIGATOR_CLASS, Wrapper};
use crate::hooks::{StyleAnchor, display_of};
use crate::nav::switch_group;
use crate::view::View;

#[cfg(doc)]
use twine_engine::ScreenAnim;

/// What a leave guard ([`Navigator::on_leave`]) decides about a navigation that would remove
/// its page.
///
/// ```
/// use twine_view::prelude::*;
///
/// let dirty = true;
/// let verdict = if dirty { Leave::Deny } else { Leave::Allow };
/// assert_eq!(verdict, Leave::Deny);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Leave {
    /// The page may be left (and deleted).
    Allow,
    /// The navigation is cancelled: nothing changes ([`NavOutcome::Denied`]).
    Deny,
}

/// The outcome of a navigation request ([`Navigator::push`], [`pop`](Navigator::pop), …).
///
/// Not `#[must_use]`: a click handler usually ignores it. Never an error the application must
/// handle — refused requests change nothing and log at `debug` (`warn` for a disposed
/// navigator).
///
/// ```
/// use twine_view::prelude::*;
///
/// assert!(NavOutcome::Accepted.is_accepted());
/// assert!(!NavOutcome::AtRoot.is_accepted());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum NavOutcome {
    /// The route stack changed (the screens follow at the next effect flush), or already was
    /// as requested ([`pop_to`](Navigator::pop_to) the current route).
    Accepted,
    /// A leave guard of a page that would be removed returned [`Leave::Deny`].
    Denied,
    /// [`pop`](Navigator::pop) / [`pop_to_root`](Navigator::pop_to_root) at the root page.
    AtRoot,
    /// [`pop_to`](Navigator::pop_to): the route is not on the stack.
    NotFound,
    /// [`reset`](Navigator::reset) with no routes (the stack is never empty).
    Empty,
    /// Requested by a leave guard while it decides about another navigation (ignored).
    Busy,
    /// The stack would be deeper than [`Limits::max_nav_depth`](twine_core::Limits::max_nav_depth):
    /// nothing changes, and a `Capacity` fault (code `NavDepth`) is raised.
    Full,
    /// The navigator's scope was disposed (logged with `warn!`).
    Disposed,
}

impl NavOutcome {
    /// Whether the request was accepted ([`NavOutcome::Accepted`]).
    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(self, NavOutcome::Accepted)
    }
}

/// Builds the page of a route on a new screen: `(engine, screen, page scope, route) -> root`.
type PageFn<R> = Rc<dyn Fn(&mut Engine, NodeId, Scope, R) -> NodeId>;

/// Lends the navigator's [`Core`] to a closure (route-independent code; see [`show`]).
type CoreAccess<'a> = &'a dyn Fn(&mut dyn FnMut(&mut Core));

/// A leave guard.
type GuardFn<R> = Box<dyn FnMut(&R) -> Leave>;

/// Transitions applied in one effect run at most (a page that navigates while it is built —
/// a redirect — takes one more); the rest waits for the next request.
const MAX_APPLY_PASSES: u32 = 4;

/// A built page: its screen, scope and focus group.
pub(crate) struct Entry {
    node: NodeId,
    scope: Scope,
    /// The page's focus group (keypad / encoder), `None` without one.
    group: Option<GroupId>,
    /// Whether the navigator created `group` (and deletes it with the page).
    owns_group: bool,
}

/// One page of the route stack (parallel to `NavState::routes`).
pub(crate) struct Page {
    /// Unique among the navigator's live pages (identifies the page to its guards).
    id: u32,
    /// The page's screen, once built.
    entry: Option<Entry>,
}

/// A registered leave guard.
struct Guard<R> {
    id: u32,
    /// The page whose removal it guards.
    page: u32,
    /// `None` while it runs.
    f: Option<GuardFn<R>>,
}

/// The route-independent state of a navigator.
struct Core {
    /// The scope owning the pages' scopes.
    scope: Scope,
    /// The pages, root first.
    pages: Vec<Page>,
    /// Pages removed from the stack whose screens are not deleted yet.
    doomed: Vec<Entry>,
    /// The animation of the transition not applied yet.
    pending: Option<ScreenLoad>,
    next_page: u32,
    next_guard: u32,
    /// Bumped by every request: wakes the transition effect, and tracked by the reads.
    tick: Signal<u32>,
    /// The navigator's anchor node: the pages inherit their text style from it (and so from
    /// the views around the navigator), although each is on a screen of its own.
    anchor: Option<NodeId>,
    /// The display of the pages and the screen shown on it before the navigator's first page
    /// (shown again when the navigator is disposed while one of its pages is active).
    home: Option<(DisplayId, Option<NodeId>)>,
    /// The default focus group before the navigator's first page.
    base_group: Option<GroupId>,
    /// A leave guard is running.
    guarding: bool,
    /// The deepest stack allowed (`Limits::max_nav_depth`, read when the navigator is built).
    max_depth: usize,
    /// Requests refused for depth since the last transition was applied (reported as a
    /// `Capacity` fault there, where the engine is lent).
    refused: u32,
    /// The first page was built (later pages get focus groups of their own).
    built_any: bool,
}

impl Core {
    fn new_page(&mut self) -> Page {
        let id = self.next_page;
        self.next_page = id.wrapping_add(1);
        Page { id, entry: None }
    }
}

struct NavState<R> {
    core: Core,
    /// The routes, root first (one per page).
    routes: Vec<R>,
    guards: Vec<Guard<R>>,
    page: PageFn<R>,
}

/// Something a request removed from the state, dropped outside its borrow (the `Drop` of a
/// route or of a guard's captures is application code).
#[expect(dead_code, reason = "held only to be dropped outside the borrow")]
enum Removed<R> {
    Route(Option<R>),
    Guard(Guard<R>),
}

/// The routes a request puts on top of the kept pages.
enum NewRoutes<R> {
    None,
    One(R),
    Many(Vec<R>),
}

/// The page context: which navigator and page a scope belongs to (for
/// [`Navigator::on_leave`]).
struct PageMark<R: 'static> {
    nav: Navigator<R>,
    page: u32,
}

impl<R> Clone for PageMark<R> {
    fn clone(&self) -> Self {
        PageMark {
            nav: self.nav,
            page: self.page,
        }
    }
}

/// A stack of typed routes shown as screens (see [`navigator`]), obtained with
/// [`expect_navigator`] (or [`use_navigator`], which returns a `Result`).
///
/// `Copy`, like a signal: move it into any number of handlers without cloning. Its state is a
/// [`StoredValue`] owned by the navigator's scope; once that scope is disposed every request
/// logs `warn!` and returns [`NavOutcome::Disposed`], and the reads return nothing (`depth`
/// is 0).
///
/// Requests change the route stack at once and are applied to the screens at the `Ui`'s next
/// effect flush (see the [module docs](mod@crate::navigator#model)), so they are safe anywhere,
/// e.g. in click handlers. The reads ([`depth`](Self::depth), [`current`](Self::current),
/// [`stack`](Self::stack)) are tracked: a label or effect reading them follows the
/// navigation.
///
/// The animation parameter of every request is a [`ScreenAnim`] or a
/// [`ScreenLoad`] (with a delay); its `auto_delete` is ignored — the navigator deletes the
/// pages that leave the stack.
pub struct Navigator<R: 'static> {
    state: StoredValue<NavState<R>>,
}

impl<R> Clone for Navigator<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for Navigator<R> {}

impl<R> PartialEq for Navigator<R> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<R> Eq for Navigator<R> {}

impl<R> core::fmt::Debug for Navigator<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let depth = self.state.try_with(|s| s.routes.len()).ok().unwrap_or(0);
        f.debug_struct("Navigator").field("depth", &depth).finish()
    }
}

/// Logs a navigator used after its scope was disposed.
#[cold]
#[inline(never)]
fn nav_disposed(what: &str) {
    twine_core::warn!(target: "twine::view", "Navigator::{} after the navigator was disposed; ignored", what);
}

/// Logs a refused request.
#[cold]
#[inline(never)]
fn nav_refused(what: &str, outcome: NavOutcome) {
    twine_core::debug!(target: "twine::view", "navigator: {} refused ({:?})", what, outcome);
}

impl<R: Clone + PartialEq + 'static> Navigator<R> {
    /// Shows the page of `route` on top of the current one with `anim`; the current page stays
    /// alive below (its state is kept for the way back).
    ///
    /// Returns [`NavOutcome::Accepted`] (or `Busy` / `Disposed`, or `Full` at
    /// [`Limits::max_nav_depth`](twine_core::Limits::max_nav_depth)). Allocates nothing once
    /// the stack's capacity has reached this depth. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { List, Item(u32) }
    ///
    /// fn list(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     button(label("Item 7")).on_click(move || nav.push(Route::Item(7), ScreenAnim::MoveLeft(Duration::ms(300))))
    /// }
    /// # let _ = list;
    /// ```
    pub fn push(&self, route: R, anim: impl Into<ScreenLoad>) -> NavOutcome {
        self.navigate("push", anim.into(), |r| Ok(r.len()), NewRoutes::One(route))
    }

    /// Goes back to the page below with `anim`. The top page's leave guards decide first; the
    /// page is deleted (its scope disposed) when the animation ends.
    ///
    /// Returns `Accepted`, `AtRoot` (one page: nothing happens), `Denied`, `Busy` or
    /// `Disposed`. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { Home, Settings }
    ///
    /// fn settings(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(Duration::ms(300))))
    /// }
    /// # let _ = settings;
    /// ```
    pub fn pop(&self, anim: impl Into<ScreenLoad>) -> NavOutcome {
        self.navigate(
            "pop",
            anim.into(),
            |r| match r.len().checked_sub(1) {
                Some(keep) if keep > 0 => Ok(keep),
                _ => Err(NavOutcome::AtRoot),
            },
            NewRoutes::None,
        )
    }

    /// Replaces the top page with the page of `route` (the old one is deleted after `anim`;
    /// its leave guards decide first). The depth stays the same.
    ///
    /// Returns `Accepted`, `Denied`, `Busy` or `Disposed`. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { Login, Home }
    ///
    /// fn login(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     // No way back to the login page.
    ///     button(label("Sign in")).on_click(move || nav.replace(Route::Home, ScreenAnim::FadeIn(Duration::ms(200))))
    /// }
    /// # let _ = login;
    /// ```
    pub fn replace(&self, route: R, anim: impl Into<ScreenLoad>) -> NavOutcome {
        self.navigate(
            "replace",
            anim.into(),
            |r| Ok(r.len().saturating_sub(1)),
            NewRoutes::One(route),
        )
    }

    /// Goes back to the topmost page whose route equals `route`, removing every page above it
    /// (their leave guards decide first, top first).
    ///
    /// Returns `Accepted` (also when `route` is already the current route: nothing to do),
    /// `NotFound`, `Denied`, `Busy` or `Disposed`. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { Home, Wizard(u8) }
    ///
    /// fn last_step(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     button(label("Restart")).on_click(move || nav.pop_to(&Route::Wizard(1), ScreenAnim::None))
    /// }
    /// # let _ = last_step;
    /// ```
    pub fn pop_to(&self, route: &R, anim: impl Into<ScreenLoad>) -> NavOutcome {
        self.navigate(
            "pop_to",
            anim.into(),
            |r| match r.iter().rposition(|x| x == route) {
                // `i < r.len()`: keeping `i + 1` routes cannot overflow.
                Some(i) => Ok(i.saturating_add(1)),
                None => Err(NavOutcome::NotFound),
            },
            NewRoutes::None,
        )
    }

    /// Goes back to the root page, removing every page above it (their leave guards decide
    /// first, top first).
    ///
    /// Returns `Accepted`, `AtRoot`, `Denied`, `Busy` or `Disposed`. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { Home, Detail(u32) }
    ///
    /// fn detail(cx: Scope, id: u32) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     button(label("Home")).on_click(move || nav.pop_to_root(ScreenAnim::MoveRight(Duration::ms(300))))
    /// }
    /// # let _ = detail;
    /// ```
    pub fn pop_to_root(&self, anim: impl Into<ScreenLoad>) -> NavOutcome {
        self.navigate(
            "pop_to_root",
            anim.into(),
            |r| {
                if r.len() > 1 {
                    Ok(1)
                } else {
                    Err(NavOutcome::AtRoot)
                }
            },
            NewRoutes::None,
        )
    }

    /// Replaces the whole stack with `stack` (root first), e.g. after a logout, or to restore
    /// a saved stack at run time ([`NavigatorView::restore`] does it before the first page is
    /// built). Every current page is removed (their leave guards decide first, top first);
    /// only the new top page is built.
    ///
    /// Returns `Accepted`, `Empty` (no routes: nothing happens), `Full` (more routes than
    /// [`Limits::max_nav_depth`](twine_core::Limits::max_nav_depth)), `Denied`, `Busy` or
    /// `Disposed`. Collects `stack` into a `Vec` (one allocation). Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { Login, Home, Profile }
    ///
    /// fn profile(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     button(label("Log out")).on_click(move || nav.reset([Route::Login], ScreenAnim::FadeIn(Duration::ms(200))))
    /// }
    /// # let _ = profile;
    /// ```
    pub fn reset(&self, stack: impl IntoIterator<Item = R>, anim: impl Into<ScreenLoad>) -> NavOutcome {
        let routes: Vec<R> = stack.into_iter().collect();
        if routes.is_empty() {
            nav_refused("reset", NavOutcome::Empty);
            return NavOutcome::Empty;
        }
        self.navigate("reset", anim.into(), |_| Ok(0), NewRoutes::Many(routes))
    }

    /// Registers a leave guard for the page that owns `cx` (the page's scope or one inside
    /// it): before a navigation removes that page ([`pop`](Self::pop),
    /// [`replace`](Self::replace), [`pop_to`](Self::pop_to),
    /// [`pop_to_root`](Self::pop_to_root), [`reset`](Self::reset); not
    /// [`push`](Self::push), which keeps it), `guard` is called with the route that would be
    /// shown, and [`Leave::Deny`] cancels the navigation ([`NavOutcome::Denied`]).
    ///
    /// The guard lives as long as `cx`. It runs untracked, while the request is made; it may
    /// open a dialog ([`ScopeExt::show_modal`](crate::ScopeExt::show_modal)) but not navigate
    /// (such requests return [`NavOutcome::Busy`]): the dialog's button navigates again once
    /// the guard allows it (e.g. after the changes were saved or discarded). Several guards of
    /// the removed pages run newest first; the first `Deny` stops.
    ///
    /// A `cx` outside the pages of this navigator logs `warn!` and registers nothing. Boxes
    /// `guard` once. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { Home, Editor }
    ///
    /// fn editor(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     let dirty = cx.signal(false);
    ///     nav.on_leave(cx, move |_to: &Route| {
    ///         if !dirty.get() {
    ///             return Leave::Allow;
    ///         }
    ///         let _ = cx.show_modal(move |_, modal: ModalHandle| {
    ///             column((
    ///                 label("Discard changes?"),
    ///                 button(label("Discard")).on_click(move || {
    ///                     dirty.set(false);
    ///                     modal.close();
    ///                     nav.pop(ScreenAnim::None);
    ///                 }),
    ///                 button(label("Keep editing")).on_click(move || modal.close()),
    ///             ))
    ///         });
    ///         Leave::Deny
    ///     });
    ///     column((
    ///         button(label("Edit")).on_click(move || dirty.set(true)),
    ///         button(label("Back")).on_click(move || nav.pop(ScreenAnim::None)),
    ///     ))
    /// }
    /// # let _ = editor;
    /// ```
    pub fn on_leave(&self, cx: Scope, guard: impl FnMut(&R) -> Leave + 'static) {
        let Some(mark) = cx.use_context::<PageMark<R>>().ok().filter(|m| m.nav == *self) else {
            twine_core::warn!(target: "twine::view", "Navigator::on_leave outside the navigator's pages; ignored");
            return;
        };
        let Some(id) = self
            .state
            .try_with_mut(|s| {
                let id = s.core.next_guard;
                s.core.next_guard = id.wrapping_add(1);
                s.guards.push(Guard {
                    id,
                    page: mark.page,
                    f: Some(Box::new(guard)),
                });
                id
            })
            .ok()
        else {
            return nav_disposed("on_leave");
        };
        let state = self.state;
        cx.on_cleanup(move || {
            let g = state
                .try_with_mut(|s| {
                    s.guards
                        .iter()
                        .position(|g| g.id == id)
                        .map(|i| s.guards.remove(i))
                })
                .ok()
                .flatten();
            drop(g); // outside the borrow
        });
    }

    /// Number of pages (0 once the navigator is disposed). Tracked.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, PartialEq, Debug)]
    /// enum Route { Home, Settings }
    ///
    /// fn header(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     label(text!("Level {}", nav.depth()))
    /// }
    /// # let _ = header;
    /// ```
    #[must_use]
    pub fn depth(&self) -> usize {
        self.read(|s| s.routes.len()).unwrap_or(0)
    }

    /// The route of the top page (`None` once the navigator is disposed). Tracked; clones the
    /// route.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, Copy, PartialEq, Debug)]
    /// enum Route { Home, Settings }
    ///
    /// fn title(cx: Scope) -> impl View {
    ///     let nav = expect_navigator::<Route>(cx);
    ///     label(move || match nav.current() {
    ///         Some(Route::Settings) => "Settings",
    ///         _ => "Home",
    ///     })
    /// }
    /// # let _ = title;
    /// ```
    #[must_use]
    pub fn current(&self) -> Option<R> {
        self.read(|s| s.routes.last().cloned()).flatten()
    }

    /// The route stack, root first (empty once the navigator is disposed): for tests and for
    /// saving the navigation state (restore it with [`NavigatorView::restore`] or
    /// [`reset`](Self::reset)). Tracked; clones the routes into a new `Vec`.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, Copy, PartialEq, Debug)]
    /// enum Route { Home, Settings }
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let view = navigator(cx, Route::Home, |_, route| label(match route {
    ///         Route::Home => "Home",
    ///         Route::Settings => "Settings",
    ///     }));
    ///     let nav = expect_navigator::<Route>(cx);
    ///     nav.push(Route::Settings, ScreenAnim::None);
    ///     assert_eq!(nav.stack(), [Route::Home, Route::Settings]);
    ///     view
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    pub fn stack(&self) -> Vec<R> {
        self.read(|s| s.routes.clone()).unwrap_or_default()
    }

    /// The depth without tracking (for logs).
    fn depth_untracked(&self) -> usize {
        self.state.try_with(|s| s.routes.len()).ok().unwrap_or(0)
    }

    /// Reads the state, tracking the navigation.
    fn read<T>(&self, f: impl FnOnce(&NavState<R>) -> T) -> Option<T> {
        let tick = self.state.try_with(|s| s.core.tick).ok()?;
        if tick.is_alive() {
            let _ = tick.get();
        }
        self.state.try_with(f).ok()
    }

    /// The common path of every request: `keep` says how many pages stay (from the bottom),
    /// `new` what goes on top of them. The guards of the removed pages decide first.
    fn navigate(
        &self,
        what: &'static str,
        load: ScreenLoad,
        keep: impl FnOnce(&[R]) -> Result<usize, NavOutcome>,
        new: NewRoutes<R>,
    ) -> NavOutcome {
        let checked = self
            .state
            .try_with(|s| {
                if s.core.guarding {
                    return Err(NavOutcome::Busy);
                }
                let keep = keep(&s.routes)?.min(s.routes.len());
                let added = match &new {
                    NewRoutes::None => 0,
                    NewRoutes::One(_) => 1,
                    NewRoutes::Many(v) => v.len(),
                };
                if keep.saturating_add(added) > s.core.max_depth {
                    return Err(NavOutcome::Full);
                }
                let removed = s.core.pages.get(keep..).unwrap_or_default();
                let guarded = s.guards.iter().any(|g| removed.iter().any(|p| p.id == g.page));
                Ok((keep, guarded, s.routes.len()))
            })
            .ok();
        let (keep, guarded, len) = match checked {
            None => {
                nav_disposed(what);
                return NavOutcome::Disposed;
            }
            Some(Err(NavOutcome::Full)) => {
                self.refuse_full(what);
                return NavOutcome::Full;
            }
            Some(Err(outcome)) => {
                nav_refused(what, outcome);
                return outcome;
            }
            Some(Ok(c)) => c,
        };
        if keep == len && matches!(new, NewRoutes::None) {
            return NavOutcome::Accepted; // already there
        }
        if guarded {
            let dest = match &new {
                NewRoutes::One(r) => Some(r.clone()),
                NewRoutes::Many(v) => v.last().cloned(),
                NewRoutes::None => self
                    .state
                    .try_with(|s| s.routes.get(keep.wrapping_sub(1)).cloned())
                    .ok()
                    .flatten(),
            };
            if let Some(dest) = dest
                && self.run_guards(keep, &dest) == Leave::Deny
            {
                nav_refused(what, NavOutcome::Denied);
                return NavOutcome::Denied;
            }
        }
        self.commit(keep, new, load);
        twine_core::debug!(target: "twine::view", "navigator: {} (depth {})", what, self.depth_untracked());
        NavOutcome::Accepted
    }

    /// A request refused at `Limits::max_nav_depth`: logged, counted and reported as a
    /// `Capacity` fault by the transition effect (woken here), which has the engine.
    #[cold]
    #[inline(never)]
    fn refuse_full(&self, what: &str) {
        twine_core::error!(target: "twine::view", "navigator: {} refused: Limits::max_nav_depth reached", what);
        let tick = self
            .state
            .try_with_mut(|s| {
                s.core.refused = s.core.refused.saturating_add(1);
                s.core.tick
            })
            .ok();
        if let Some(tick) = tick.filter(Signal::is_alive) {
            tick.update(|t| *t = t.wrapping_add(1));
        }
    }

    /// Runs the guards of the pages above `keep`, newest first, until one denies.
    fn run_guards(&self, keep: usize, dest: &R) -> Leave {
        let Some(rt) = self
            .state
            .try_with_mut(|s| {
                s.core.guarding = true;
                s.core.scope.runtime()
            })
            .ok()
        else {
            return Leave::Allow;
        };
        let mut below: Option<u32> = None;
        let mut verdict = Leave::Allow;
        while verdict == Leave::Allow {
            let next = self
                .state
                .try_with_mut(|s| {
                    let removed = s.core.pages.get(keep..).unwrap_or_default();
                    let g = s
                        .guards
                        .iter_mut()
                        .filter(|g| below.is_none_or(|b| g.id < b) && removed.iter().any(|p| p.id == g.page))
                        .max_by_key(|g| g.id)?;
                    Some((g.id, g.f.take()))
                })
                .ok()
                .flatten();
            let Some((id, f)) = next else { break };
            below = Some(id);
            let Some(mut f) = f else { continue };
            verdict = rt.untrack(|| f(dest));
            // Put it back (dropped outside the borrow if its scope went away meanwhile).
            let orphan = self
                .state
                .try_with_mut(move |s| match s.guards.iter_mut().find(|g| g.id == id) {
                    Some(g) => {
                        g.f = Some(f);
                        None
                    }
                    None => Some(f),
                })
                .ok()
                .flatten();
            drop(orphan);
        }
        self.state.try_with_mut(|s| s.core.guarding = false).ok();
        verdict
    }

    /// Changes the stack: the pages above `keep` leave (their screens are doomed), `new` goes
    /// on top, and the transition effect is woken.
    fn commit(&self, keep: usize, new: NewRoutes<R>, load: ScreenLoad) {
        // One item at a time, dropped outside the borrow.
        while let Some(item) = self.state.try_with_mut(|s| take_removed(s, keep)).ok().flatten() {
            drop(item);
        }
        let Some(tick) = self
            .state
            .try_with_mut(move |s| {
                let add = |s: &mut NavState<R>, r: R| {
                    let page = s.core.new_page();
                    push_page(s, page, r);
                };
                match new {
                    NewRoutes::None => {}
                    NewRoutes::One(r) => add(s, r),
                    NewRoutes::Many(v) => {
                        for r in v {
                            add(s, r);
                        }
                    }
                }
                s.core.pending = Some(load);
                s.core.tick
            })
            .ok()
        else {
            return nav_disposed("navigate");
        };
        if tick.is_alive() {
            tick.update(|t| *t = t.wrapping_add(1));
        }
    }

    /// Applies the pending transition (engine required), after reporting the requests
    /// refused for depth.
    fn apply(&self, e: &mut Engine) {
        let refused = self
            .state
            .try_with_mut(|s| core::mem::take(&mut s.core.refused))
            .unwrap_or(0);
        if refused > 0 {
            e.raise_fault(
                twine_engine::FaultRecord::new(twine_core::fault::FaultKind::Capacity)
                    .occurrences(refused)
                    .code(twine_core::fault::CapacityFault::NavDepth.code()),
            );
        }
        for _ in 0..MAX_APPLY_PASSES {
            let Some(Some(load)) = self.state.try_with_mut(|s| s.core.pending.take()).ok() else {
                return;
            };
            let Some(display) = self
                .state
                .try_with(|s| s.core.scope)
                .ok()
                .and_then(|c| display_of(c, e))
            else {
                return;
            };
            // The screen the transition leaves is the active one once a running load ended.
            e.finish_screen_anim(display);
            self.build_top(e, display);
            if self.state.try_with(|s| s.core.pending.is_some()).ok() != Some(false) {
                continue; // the page navigated while it was built: go on with that
            }
            let top = self
                .state
                .try_with(|s| {
                    s.core
                        .pages
                        .last()
                        .and_then(|p| p.entry.as_ref().map(|en| (en.node, en.group)))
                })
                .ok()
                .flatten();
            show(e, display, top, load, &|f| {
                self.state.try_with_mut(|s| f(&mut s.core)).ok();
            });
            twine_core::debug!(target: "twine::view", "navigator: depth {}", self.depth_untracked());
        }
        if self.state.try_with(|s| s.core.pending.is_some()).ok() == Some(true) {
            twine_core::warn!(target: "twine::view", "navigator: pages kept navigating while built; the rest waits for the next request");
        }
    }

    /// Builds the top page if it is not built yet (the first page with the current focus
    /// group, later ones with a group of their own).
    fn build_top(&self, e: &mut Engine, display: DisplayId) {
        let Some(Some((id, route, page, scope, anchor, own_group))) = self
            .state
            .try_with(|s| {
                let top = s.core.pages.last()?;
                if top.entry.is_some() {
                    return None;
                }
                Some((
                    top.id,
                    s.routes.last()?.clone(),
                    s.page.clone(),
                    s.core.scope,
                    s.core.anchor,
                    s.core.built_any,
                ))
            })
            .ok()
        else {
            return;
        };
        let mark = PageMark { nav: *self, page: id };
        let mut route = Some(route);
        let entry = build_entry(e, display, scope, anchor, own_group, &mut |e, screen, child| {
            child.provide(mark.clone());
            route
                .take()
                .map_or(twine_engine::DEAD_NODE, |r| page(e, screen, child, r))
        });
        let mut entry = entry;
        self.state
            .try_with_mut(|s| {
                s.core.built_any = true;
                if let Some(en) = entry.take() {
                    match s.core.pages.iter_mut().find(|p| p.id == id) {
                        Some(p) => p.entry = Some(en),
                        None => twine_core::alloc_guard::push(
                            &mut s.core.doomed,
                            en,
                            twine_core::alloc_guard::AllocSite::NavStack,
                        ), // removed while it was built
                    }
                }
            })
            .ok();
        // The navigator went away while the page was built: nothing will show it.
        if let Some(en) = entry
            && e.tree().contains(en.node)
        {
            let _ = e.delete(en.node);
        }
    }
}

/// Appends a page and its route (the two stacks stay parallel), noting growth for the
/// allocation guard.
fn push_page<R>(s: &mut NavState<R>, page: Page, route: R) {
    use twine_core::alloc_guard::{self, AllocSite};
    alloc_guard::push(&mut s.core.pages, page, AllocSite::NavStack);
    alloc_guard::push(&mut s.routes, route, AllocSite::NavStack);
}

/// Takes one thing the removal of the pages above `keep` drops: a guard of a removed page, or
/// the top page (its built screen is doomed).
fn take_removed<R>(s: &mut NavState<R>, keep: usize) -> Option<Removed<R>> {
    let removed = s.core.pages.get(keep..).unwrap_or_default();
    if let Some(i) = s
        .guards
        .iter()
        .position(|g| removed.iter().any(|p| p.id == g.page))
    {
        return Some(Removed::Guard(s.guards.remove(i)));
    }
    if s.core.pages.len() > keep {
        if let Some(Page { entry: Some(en), .. }) = s.core.pages.pop() {
            twine_core::alloc_guard::push(
                &mut s.core.doomed,
                en,
                twine_core::alloc_guard::AllocSite::NavStack,
            );
        }
        return Some(Removed::Route(s.routes.pop()));
    }
    None
}

/// Builds a page on a new screen of `display` with `build` (the screen's scope is a child of
/// `parent`, disposed when the screen is deleted). With `own_group` the page gets a focus
/// group of its own, which becomes the default group and the keypads' and encoders' group.
fn build_entry(
    e: &mut Engine,
    display: DisplayId,
    parent: Scope,
    anchor: Option<NodeId>,
    own_group: bool,
    build: &mut dyn FnMut(&mut Engine, NodeId, Scope) -> NodeId,
) -> Option<Entry> {
    let rt = parent.runtime();
    let Ok(screen) = e.create_screen(display) else {
        twine_core::warn!(target: "twine::view", "navigator: cannot create a screen");
        return None;
    };
    let (group, owns_group) = if own_group {
        let prev = e.default_group();
        match e.create_group() {
            Ok(g) => {
                switch_group(e, prev, Some(g));
                (Some(g), true)
            }
            Err(_) => (prev, false),
        }
    } else {
        (e.default_group(), false)
    };
    let child = parent.child();
    let root = build(e, screen, child);
    if let Some(a) = anchor.filter(|a| e.tree().contains(*a)) {
        e.set_style_parent(root, Some(a));
    }
    child.provide(StyleAnchor::new(root));
    on_delete(rt, e, screen, move || child.dispose());
    twine_core::debug!(target: "twine::view", "navigator: page {} built", fmt_node_id(screen));
    Some(Entry {
        node: screen,
        scope: child,
        group,
        owns_group,
    })
}

/// Shows the top page `(screen, group)` with `load` and deletes the doomed pages: the one
/// being left when the animation ends (`auto_delete`), the others at once. `core` lends the
/// navigator's state (never across engine calls that run application code).
fn show(
    e: &mut Engine,
    display: DisplayId,
    top: Option<(NodeId, Option<GroupId>)>,
    load: ScreenLoad,
    core: CoreAccess<'_>,
) {
    let active = e.active_screen(display);
    let mut auto_delete = false;
    if let Some((node, group)) = top {
        switch_group(e, e.default_group(), group);
        core(&mut |c| auto_delete = c.doomed.iter().any(|d| Some(d.node) == active));
        if Some(node) != active {
            e.load_screen_anim(node, ScreenLoad { auto_delete, ..load });
        }
    } else {
        twine_core::warn!(target: "twine::view", "navigator: the top page could not be built");
    }
    let mut still_shown = None;
    loop {
        let mut next = None;
        core(&mut |c| next = c.doomed.pop());
        let Some(d) = next else { break };
        if Some(d.node) == active && top.is_none() {
            still_shown = Some(d); // nothing replaced it: deleted by a later transition
            continue;
        }
        if d.owns_group
            && let Some(g) = d.group.filter(|g| e.default_group() != Some(*g))
        {
            e.delete_group(g);
        }
        if Some(d.node) == active && auto_delete {
            continue; // deleted by the engine at the end of the load
        }
        if e.tree().contains(d.node) {
            let _ = e.delete(d.node); // disposes the page's scope (`on_delete`)
        } else {
            d.scope.dispose();
        }
    }
    if still_shown.is_some() {
        core(&mut |c| c.doomed.extend(still_shown.take()));
    }
}

/// What a disposed navigator leaves to the engine: its screens and focus groups.
pub(crate) struct NavTeardown {
    home: Option<(DisplayId, Option<NodeId>)>,
    base_group: Option<GroupId>,
    pages: Vec<Page>,
    doomed: Vec<Entry>,
}

impl NavTeardown {
    /// Deletes the navigator's screens and focus groups; shows the screen shown before the
    /// navigator again if one of its pages is active.
    pub(crate) fn apply(self, e: &mut Engine) {
        let entries = || {
            self.pages
                .iter()
                .filter_map(|p| p.entry.as_ref())
                .chain(self.doomed.iter())
        };
        let ours = |n: Option<NodeId>| n.is_some_and(|n| entries().any(|en| en.node == n));
        if let Some((d, home)) = self.home {
            e.finish_screen_anim(d);
            if ours(e.active_screen(d))
                && let Some(home) = home.filter(|h| e.tree().contains(*h))
            {
                e.load_screen(home);
            }
        }
        let def = e.default_group();
        if entries().any(|en| en.owns_group && en.group == def && def.is_some()) {
            switch_group(e, def, self.base_group);
        }
        for en in entries() {
            if en.owns_group
                && let Some(g) = en.group
            {
                e.delete_group(g);
            }
            if e.tree().contains(en.node) {
                let _ = e.delete(en.node);
            }
        }
    }
}

/// A navigation root over routes of type `R`: provides a [`Navigator<R>`](Navigator) in `cx`
/// (see [`expect_navigator`]) and shows the page of `initial` as the display's active screen.
/// `page(scope, route)` builds the view of a route's page; it is called once per page, with
/// the page's own scope, when the page is first shown. The view itself is an empty anchor
/// node (the pages inherit their text style from it).
///
/// `R` is the application's route type: `Clone` (the page function receives its own copy)
/// and `PartialEq` ([`Navigator::pop_to`]); usually an enum whose variants carry what a page
/// needs (`Route::Detail(id)`). When the pages have different view types, return
/// [`AnyView`](crate::AnyView)s (`.into_any()`) from the `match`.
///
/// **Saving and restoring the stack** (e.g. across a reset of the device) needs no
/// serialization framework: read [`Navigator::stack`], encode the routes however the
/// application likes (a byte per variant, `postcard`, …), and give the decoded routes to
/// [`NavigatorView::restore`] at the next start.
///
/// Boxes `page` once; see the [module docs](mod@crate::navigator) for the model and the memory
/// cost. Never panics.
///
/// ```
/// use twine_view::prelude::*;
///
/// #[derive(Clone, PartialEq, Debug)]
/// enum Route {
///     Home,
///     Detail(u32),
/// }
///
/// fn app(cx: Scope) -> impl View {
///     navigator(cx, Route::Home, |cx, route| match route {
///         Route::Home => home(cx).into_any(),
///         Route::Detail(id) => detail(cx, id).into_any(),
///     })
/// }
///
/// fn home(cx: Scope) -> impl View {
///     let nav = expect_navigator::<Route>(cx);
///     button(label("Open 42")).on_click(move || nav.push(Route::Detail(42), ScreenAnim::MoveLeft(Duration::ms(300))))
/// }
///
/// fn detail(cx: Scope, id: u32) -> impl View {
///     let nav = expect_navigator::<Route>(cx);
///     column((
///         label(text!("Item {}", id)),
///         button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(Duration::ms(300)))),
///     ))
/// }
/// # let _ = app;
/// ```
pub fn navigator<R, V>(cx: Scope, initial: R, page: impl Fn(Scope, R) -> V + 'static) -> NavigatorView<R>
where
    R: Clone + PartialEq + 'static,
    V: View,
{
    let rt = cx.runtime();
    let page: PageFn<R> = Rc::new(move |e: &mut Engine, screen: NodeId, child: Scope, route: R| {
        let view = EngineAccess::provide(rt, e, || rt.untrack(|| page(child, route)));
        let mut bcx = BuildCx::new(e, screen, child);
        view.build(&mut bcx)
    });
    let mut core = Core {
        scope: cx,
        pages: Vec::new(),
        doomed: Vec::new(),
        pending: None,
        next_page: 0,
        next_guard: 0,
        tick: cx.signal(0),
        anchor: None,
        home: None,
        base_group: None,
        guarding: false,
        max_depth: usize::from(twine_core::Limits::DEFAULT.max_nav_depth),
        refused: 0,
        built_any: false,
    };
    let root = core.new_page();
    core.pages.push(root);
    let routes = alloc::vec![initial];
    let nav = Navigator {
        state: cx.stored_value(NavState {
            core,
            routes,
            guards: Vec::new(),
            page,
        }),
    };
    cx.provide(nav);
    // The pages' screens are deleted with the navigator (its scope's cleanup runs before
    // its state is dropped).
    cx.on_cleanup(move || {
        let Some(t) = nav
            .state
            .try_with_mut(|s| NavTeardown {
                home: s.core.home,
                base_group: s.core.base_group,
                pages: core::mem::take(&mut s.core.pages),
                doomed: core::mem::take(&mut s.core.doomed),
            })
            .ok()
        else {
            return;
        };
        if t.pages.iter().all(|p| p.entry.is_none()) && t.doomed.is_empty() {
            return;
        }
        let mut t = Some(t);
        if EngineAccess::with(rt, |e| {
            if let Some(t) = t.take() {
                t.apply(e);
            }
        })
        .is_none()
            && let Some(t) = t
        {
            defer(cx, EngineCmd::nav_teardown(Box::new(t)));
        }
    });
    NavigatorView { nav }
}

/// The view of [`navigator`]: an anchor node, plus [`restore`](Self::restore).
#[must_use]
pub struct NavigatorView<R: 'static> {
    nav: Navigator<R>,
}

impl<R> core::fmt::Debug for NavigatorView<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigatorView").field("nav", &self.nav).finish()
    }
}

impl<R: Clone + PartialEq + 'static> NavigatorView<R> {
    /// Starts with `stack` (root first) instead of the initial route, e.g. a stack saved
    /// before a reset of the device. Only the top page is built; the pages below are built
    /// when they are shown. An empty `stack` keeps the initial route. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// #[derive(Clone, Copy, PartialEq, Debug)]
    /// enum Route { Home, Settings, Wifi }
    ///
    /// impl Route {
    ///     // The application's own encoding (no serialization framework needed).
    ///     fn to_byte(self) -> u8 { self as u8 }
    ///     fn from_byte(b: u8) -> Option<Route> {
    ///         [Route::Home, Route::Settings, Route::Wifi].get(usize::from(b)).copied()
    ///     }
    /// }
    ///
    /// fn page(_cx: Scope, route: Route) -> impl View {
    ///     label(match route {
    ///         Route::Home => "Home",
    ///         Route::Settings => "Settings",
    ///         Route::Wifi => "Wi-Fi",
    ///     })
    /// }
    ///
    /// // Saved earlier: `nav.stack().into_iter().map(Route::to_byte)` into retained RAM or flash.
    /// static SAVED: [u8; 3] = [0, 1, 2];
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     navigator(cx, Route::Home, page).restore(SAVED.iter().filter_map(|b| Route::from_byte(*b)))
    /// }
    /// # let _ = (app, Route::to_byte);
    /// ```
    pub fn restore(self, stack: impl IntoIterator<Item = R>) -> Self {
        let mut stack = stack.into_iter().peekable();
        if stack.peek().is_none() {
            return self;
        }
        let nav = self.nav;
        while let Some(item) = nav.state.try_with_mut(|s| take_removed(s, 0)).ok().flatten() {
            drop(item);
        }
        for r in stack {
            nav.state
                .try_with_mut(move |s| {
                    let page = s.core.new_page();
                    push_page(s, page, r);
                })
                .ok();
        }
        self
    }
}

impl<R: Clone + PartialEq + 'static> View for NavigatorView<R> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let anchor = cx.create(Wrapper(&NAVIGATOR_CLASS));
        if anchor == twine_engine::DEAD_NODE {
            return anchor; // not created (reported): build no pages for it
        }
        let nav = self.nav;
        let scope = cx.scope();
        let rt = scope.runtime();
        let e = cx.engine();
        let display = display_of(nav.state.with(|s| s.core.scope), e);
        let (home, base_group) = (display.map(|d| (d, e.active_screen(d))), e.default_group());
        let limits = *e.limits();
        nav.state.with_mut(|s| {
            s.core.anchor = Some(anchor);
            s.core.home = home;
            s.core.base_group = base_group;
            s.core.max_depth = usize::from(limits.max_nav_depth);
            if limits.preallocate {
                let n = s.core.max_depth;
                s.core.pages.reserve_exact(n.saturating_sub(s.core.pages.len()));
                s.routes.reserve_exact(n.saturating_sub(s.routes.len()));
                // The pages waiting to be deleted: every page of a full stack, plus the one
                // still shown during a transition.
                let doomed = n.saturating_add(1);
                s.core
                    .doomed
                    .reserve_exact(doomed.saturating_sub(s.core.doomed.len()));
            }
        });
        // A restored stack deeper than the limit keeps its lower pages.
        let max = nav.state.with(|s| s.core.max_depth);
        let mut cut = 0u32;
        while let Some(item) = nav.state.try_with_mut(|s| take_removed(s, max)).ok().flatten() {
            if matches!(item, Removed::Route(_)) {
                cut = cut.saturating_add(1);
            }
            drop(item);
        }
        if cut > 0 {
            twine_core::error!(target: "twine::view", "navigator: restored stack cut to Limits::max_nav_depth = {}", max);
            e.raise_fault(
                twine_engine::FaultRecord::new(twine_core::fault::FaultKind::Capacity)
                    .occurrences(cut)
                    .code(twine_core::fault::CapacityFault::NavDepth.code()),
            );
        }
        if let Some(display) = display {
            nav.build_top(e, display);
            if let Some(node) = nav.state.with(|s| {
                s.core
                    .pages
                    .last()
                    .and_then(|p| p.entry.as_ref().map(|en| en.node))
            }) {
                e.load_screen(node);
            }
        }
        let tick = nav.state.with(|s| s.core.tick);
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                tick.get();
                if nav
                    .state
                    .try_with(|s| s.core.pending.is_none() && s.core.refused == 0)
                    .ok()
                    .unwrap_or(true)
                {
                    return;
                }
                if EngineAccess::with(rt, |e| nav.apply(e)).is_none() {
                    rt.defer_current_effect();
                }
            });
        });
        anchor
    }
}

/// The [`Navigator`] over routes of type `R` provided by the enclosing [`navigator`] (the
/// innermost one for nested navigators of the same route type), or [`MissingContext`] naming
/// `Navigator<R>` when `cx` is not inside one — the non-panicking form (see
/// [`expect_navigator`] for application pages, and [`Scope`] § Context values for the
/// convention and the lookup cost: a scope-chain walk, allocation-free). Look it up once when
/// the page is built and move the `Copy` handle into its handlers.
///
/// Never panics.
///
/// ```
/// use twine_view::prelude::*;
///
/// #[derive(Clone, PartialEq, Debug)]
/// enum Route { Home, Details }
///
/// // A component usable inside and outside a navigator: "Back" only where it can go back.
/// fn toolbar(cx: Scope) -> impl View {
///     let back = use_navigator::<Route>(cx).ok();
///     row((
///         label("Twine"),
///         when(move || back.is_some_and(|nav| nav.depth() > 1), move |_| {
///             button(label("Back")).on_click(move || {
///                 if let Some(nav) = back {
///                     nav.pop(ScreenAnim::None);
///                 }
///             })
///         }),
///     ))
/// }
/// # let _ = toolbar;
/// ```
pub fn use_navigator<R: 'static>(cx: Scope) -> Result<Navigator<R>, MissingContext> {
    cx.use_context::<Navigator<R>>()
}

/// The [`Navigator`] over routes of type `R` provided by the enclosing [`navigator`]: the
/// panicking form of [`use_navigator`], a thin `#[track_caller]` wrapper for pages, which are
/// always inside their navigator.
///
/// Not available with the `safety` feature (the profile has no panicking context lookups:
/// use [`use_navigator`] and handle the error).
///
/// # Panics
///
/// If `cx` is not inside a navigator over `R`, with `"context
/// twine_view::navigator::Navigator<{R}> not provided"` at the caller's location
/// ([`MissingContext`]).
///
/// ```
/// use twine_view::prelude::*;
///
/// #[derive(Clone, PartialEq, Debug)]
/// enum Route { Home, Details }
///
/// fn details(cx: Scope) -> impl View {
///     let nav = expect_navigator::<Route>(cx); // inside the navigator's pages
///     button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(Duration::ms(300))))
/// }
/// # let _ = details;
/// ```
#[cfg(not(feature = "safety"))]
#[must_use]
#[track_caller]
pub fn expect_navigator<R: 'static>(cx: Scope) -> Navigator<R> {
    cx.expect_context::<Navigator<R>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_cost_matches_the_docs() {
        // The module docs promise about `size_of::<R>() + 32` bytes per page of the stack.
        assert!(size_of::<Page>() <= 32, "{}", size_of::<Page>());
    }
}
