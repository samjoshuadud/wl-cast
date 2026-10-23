use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::wl_registry::{self, WlRegistry},
};

struct State;

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        _state: &mut Self,
        _registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            println!("{name:>3} {interface} v{version}");
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();

    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut State)?;

    Ok(())
}
