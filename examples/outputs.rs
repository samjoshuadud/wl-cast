use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{
        wl_output::{self, WlOutput},
        wl_registry::{self, WlRegistry},
    },
};

#[derive(Default)]
struct Monitor {
    name: String,
    width: i32,
    height: i32,
    refresh_mhz: i32,
    scale: i32,
}

#[derive(Default)]
struct State {
    monitors: Vec<Monitor>,
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "wl_output" {
                let index = state.monitors.len();
                registry.bind::<WlOutput, _, _>(name, version.min(4), qh, index);
                state.monitors.push(Monitor::default());
            }
        }
    }
}

impl Dispatch<WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _output: &WlOutput,
        event: wl_output::Event,
        index: &usize,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let monitor = &mut state.monitors[*index];
        match event {
            wl_output::Event::Name { name } => monitor.name = name,
            wl_output::Event::Mode {
                width,
                height,
                refresh,
                ..
            } => {
                monitor.width = width;
                monitor.height = height;
                monitor.refresh_mhz = refresh;
            }
            wl_output::Event::Scale { factor } => monitor.scale = factor,
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let mut state = State::default();

    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;

    for monitor in &state.monitors {
        println!(
            "{} {}x{} @ {}Hz scale {}",
            monitor.name,
            monitor.width,
            monitor.height,
            monitor.refresh_mhz / 1000,
            monitor.scale
        );
    }

    Ok(())
}
