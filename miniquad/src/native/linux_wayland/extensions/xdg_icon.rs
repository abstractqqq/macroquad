// xdg-toplevel-icon-v1.xml

use crate::wayland_interface;

use super::{
    super::libwayland_client::{wl_interface, wl_message},
    wayland_protocol::wl_buffer_interface,
    xdg_shell::xdg_toplevel_interface,
};

#[rustfmt::skip]
wayland_interface!(
    xdg_toplevel_icon_manager_v1_interface,
    xdg_toplevel_icon_manager_v1,
    1,
    [
        (destroy, "", ()),
        (create_icon, "n", (xdg_toplevel_icon_v1_interface)),
        (set_icon, "o?o", (xdg_toplevel_interface, xdg_toplevel_icon_v1_interface))
    ],
    [("icon_size", "i"), ("done", "")]
);

#[rustfmt::skip]
wayland_interface!(
    xdg_toplevel_icon_v1_interface,
    xdg_toplevel_icon_v1,
    1,
    [
        (destroy, "", ()),
        (set_name, "s", ()),
        (add_buffer, "oi", (wl_buffer_interface))
    ],
    []
);

crate::wl_listener!(
    xdg_toplevel_icon_manager_v1_listener,
    xdg_toplevel_icon_manager_v1,
    xdg_toplevel_icon_manager_v1_dummy,
    fn icon_size(size: core::ffi::c_int),
    fn done(),
);

