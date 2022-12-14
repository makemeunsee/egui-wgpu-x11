use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::shape;
use x11rb::protocol::xfixes::{
    destroy_region, ConnectionExt as _, RegionWrapper, SetWindowShapeRegionRequest,
};
use x11rb::protocol::xproto::{
    ClientMessageEvent, ColormapAlloc, ColormapWrapper, ConfigureWindowAux, ConnectionExt as _,
    CreateWindowAux, EventMask, Screen, StackMode, Window, WindowClass,
};

pub fn xfixes_init<Conn>(conn: &Conn)
where
    Conn: Connection,
{
    conn.xfixes_query_version(100, 0).unwrap();
}

/// from <https://stackoverflow.com/a/33735384>
pub fn input_passthrough<Conn>(conn: &Conn, win_id: u32) -> Result<()>
where
    Conn: Connection,
{
    let rw = RegionWrapper::create_region(conn, &[])?;

    let set_shape_request = SetWindowShapeRegionRequest {
        dest: win_id,
        dest_kind: shape::SK::BOUNDING,
        x_offset: 0,
        y_offset: 0,
        region: 0,
    };
    conn.send_trait_request_without_reply(set_shape_request)?;

    let set_shape_request = SetWindowShapeRegionRequest {
        dest: win_id,
        dest_kind: shape::SK::INPUT,
        x_offset: 0,
        y_offset: 0,
        region: rw.region(),
    };
    conn.send_trait_request_without_reply(set_shape_request)?;

    // TODO: does not fail but now triggers an error event, though it did not when it was inlined in main, ??
    destroy_region(conn, rw.region())?;

    Ok(())
}

/// from <https://stackoverflow.com/a/16235920>
/// possible alt: <https://github.com/libsdl-org/SDL/blob/85e6500065bbe37e9131c0ff9cd7e5af6d256730/src/video/x11/SDL_x11window.c#L153-L175>
pub fn always_on_top<Conn>(conn: &Conn, root_win_id: u32, win_id: u32) -> Result<()>
where
    Conn: Connection,
{
    let wm_state = conn
        .intern_atom(false, "_NET_WM_STATE".as_bytes())?
        .reply()?
        .atom;
    let wm_state_above = conn
        .intern_atom(false, "_NET_WM_STATE_ABOVE".as_bytes())?
        .reply()?
        .atom;

    const _NET_WM_STATE_ADD: u32 = 1;
    let event_always_on_top = ClientMessageEvent::new(
        32,
        win_id,
        wm_state,
        [_NET_WM_STATE_ADD, wm_state_above, 0, 0, 0],
    );
    conn.send_event(
        false,
        root_win_id,
        EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
        event_always_on_top,
    )?;

    Ok(())
}

/// original hack, as `always_on_top` patterns are not fully effective with Xmonad
/// not tested on other WMs yet
pub fn raise_if_not_top<Conn>(conn: &Conn, root_win_id: u32, win_id: u32) -> Result<()>
where
    Conn: Connection,
{
    let tree = conn.query_tree(root_win_id)?.reply()?.children;
    // runs on the assumption that the top most window is the last of the root's children
    if tree.last() != Some(&win_id) {
        let values = ConfigureWindowAux::default().stack_mode(StackMode::ABOVE);
        conn.configure_window(win_id, &values)?;
    }

    Ok(())
}

pub fn create_overlay_window<Conn>(
    conn: &Conn,
    screen: &Screen,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
) -> Result<Window>
where
    Conn: Connection,
{
    let depths = &screen.allowed_depths;
    let visuals = &depths.iter().find(|&d| d.depth == 32).unwrap().visuals;

    let cw = ColormapWrapper::create_colormap(
        conn,
        ColormapAlloc::NONE,
        screen.root,
        visuals.first().unwrap().visual_id,
    )?;

    let win_id = conn.generate_id()?;

    conn.create_window(
        32,
        win_id,
        screen.root,
        x,
        y,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        visuals.first().unwrap().visual_id,
        &CreateWindowAux::new()
            .background_pixel(0x00000000)
            .colormap(Some(cw.into_colormap()))
            .override_redirect(Some(1))
            .border_pixel(Some(1))
            .event_mask(Some(0b1_1111_1111_1111_1111_1111_1111u32.into())),
    )?;

    input_passthrough(conn, win_id)?;

    always_on_top(conn, screen.root, win_id)?;

    Ok(win_id)
}
