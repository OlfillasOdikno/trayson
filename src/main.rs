use async_std::channel;
use futures_util::{stream, try_join};
use serde::Serialize;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashSet, HashMap};
use std::env::temp_dir;
use std::error::Error;
use std::hash::{Hash, Hasher};
use zbus::zvariant::ObjectPath;
use zbus::{
    dbus_interface, dbus_proxy, export::futures_util::StreamExt, Connection, ConnectionBuilder,
    SignalContext,
};

#[derive(Debug, Clone, Serialize)]
struct Icon {
    width: usize,
    height: usize,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct Item {
    title: String,
    icon: Icon,
}

//https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/StatusNotifierItem/
#[dbus_proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem",
    gen_async = true
)]
trait StatusNotifierItem {
    #[dbus_proxy(property)]
    fn category(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn id(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn title(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn status(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn window_id(&self) -> zbus::Result<u32>;

    #[dbus_proxy(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[dbus_proxy(property)]
    fn overlay_icon_name(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn overlay_icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[dbus_proxy(property)]
    fn attention_icon_name(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn attention_icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[dbus_proxy(property)]
    fn attention_movie_name(&self) -> zbus::Result<String>;

    #[dbus_proxy(property)]
    fn tool_tip(&self) -> zbus::Result<(String, Vec<(i32, i32, Vec<u8>)>, String, String)>;

    #[dbus_proxy(property)]
    fn item_is_menu(&self) -> zbus::Result<bool>;

    #[dbus_proxy(property)]
    fn menu(&self) -> zbus::Result<ObjectPath<'_>>;

    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;

    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    fn scroll(&self, delta: &i32, orientation: String) -> zbus::Result<()>;

    #[dbus_proxy(signal)]
    fn new_title(&self) -> zbus::Result<()>;

    #[dbus_proxy(signal)]
    fn new_icon(&self) -> zbus::Result<()>;

    #[dbus_proxy(signal)]
    fn new_attention_icon(&self) -> zbus::Result<()>;

    #[dbus_proxy(signal)]
    fn new_overlay_icon(&self) -> zbus::Result<()>;

    #[dbus_proxy(signal)]
    fn new_tool_tip(&self) -> zbus::Result<()>;

    #[dbus_proxy(signal)]
    fn new_status(&self, status: String) -> zbus::Result<()>;
}

#[dbus_proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    #[dbus_proxy(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;

    #[dbus_proxy(signal)]
    fn status_notifier_item_registered(&self, service: &str) -> zbus::Result<()>;
}

struct StatusNotifierWatcher {
    registered: bool,
    items: HashSet<String>,
}

#[dbus_interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    #[dbus_interface(signal)]
    async fn status_notifier_item_registered(
        ctxt: &SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[dbus_interface(signal)]
    async fn status_notifier_item_unregistered(
        ctxt: &SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[dbus_interface(signal)]
    async fn status_notifier_host_registered(
        ctxt: &SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    async fn register_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        self.items.insert(service.to_string());
        self.registered_status_notifier_items_changed(&ctxt).await?;
        StatusNotifierWatcher::status_notifier_item_registered(&ctxt, service).await?;
        Ok(())
    }

    async fn unregister_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        self.items.remove(&service.to_string());
        self.registered_status_notifier_items_changed(&ctxt).await?;
        StatusNotifierWatcher::status_notifier_item_unregistered(&ctxt, service).await?;
        Ok(())
    }

    async fn register_status_notifier_host(
        &mut self,
        service: &str,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        self.registered = true;
        self.is_status_notifier_host_registered_changed(&ctxt)
            .await?;
        StatusNotifierWatcher::status_notifier_host_registered(&ctxt, service).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn protocol_version(&self) -> u64 {
        1
    }

    #[dbus_interface(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        self.registered
    }

    #[dbus_interface(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.clone().into_iter().collect::<Vec<_>>()
    }
}

struct StatusNotifierHost {}
#[dbus_interface(name = "org.kde.StatusNotifierHost-eww")] //TODO make unique
impl StatusNotifierHost {}

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let watcher = StatusNotifierWatcher {
        registered: false,
        items: HashSet::new(),
    };
    let host = StatusNotifierHost {};

    let c1 = ConnectionBuilder::session()?
        .name("org.kde.StatusNotifierWatcher")?
        .serve_at("/StatusNotifierWatcher", watcher)?
        .build()
        .await?;

    let c2 = ConnectionBuilder::session()?
        .name("org.kde.StatusNotifierHost-eww")?
        .serve_at("/StatusNotifierHost", host)?
        .build()
        .await?;

    let m = c1
        .call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.kde.StatusNotifierWatcher"),
            "RegisterStatusNotifierHost",
            &("org.kde.StatusNotifierHost-eww"),
        )
        .await?;

    let reply: () = m.body().unwrap();

    let proxy = StatusNotifierWatcherProxy::builder(&c1)
        .cache_properties(zbus::CacheProperties::No)
        .build()
        .await
        .unwrap();

    let stream = proxy
        .receive_status_notifier_item_registered()
        .await
        .unwrap();

    let (s, r) = channel::unbounded();
    let (s2, r2) = channel::unbounded();

    let task1 = stream
        .map(|signal| (s.clone(), s2.clone(), signal))
        .for_each_concurrent(None, |(s, s2, signal)| async move {
            if let Some(args) = signal.args().ok() {
                let c3 = ConnectionBuilder::session().unwrap().build().await.unwrap();

                let proxy = StatusNotifierItemProxy::builder(&c3)
                    .cache_properties(zbus::CacheProperties::No)
                    .destination(args.service)
                    .unwrap()
                    .build()
                    .await
                    .unwrap();

                let mut owner_change = proxy.receive_owner_changed().await.unwrap();
                let signals = proxy.receive_all_signals().await.unwrap();
                try_join!(
                    async {
                        let title = proxy.get_property::<String>("Title").await.unwrap();
                        let icon = &proxy
                            .get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap")
                            .await
                            .unwrap()[0];

                        let iter = stream::iter(icon.2.chunks_exact(4));
                        let img = iter
                            .map(|pixel| [pixel[1], pixel[2], pixel[3], pixel[0]])
                            .fold(Vec::new(), |mut state, x| async move {
                                state.extend(x);
                                state
                            })
                            .await;

                        let mut temp_dir = temp_dir();
                        let mut hasher = DefaultHasher::new();
                        Hash::hash_slice(&img, &mut hasher);
                        temp_dir.push(format!("{:x}.png", hasher.finish()));

                        let a = image::RgbaImage::from_vec(
                            u32::try_from(icon.0).unwrap(),
                            u32::try_from(icon.1).unwrap(),
                            img,
                        )
                        .unwrap();
                        a.save(temp_dir.to_str().unwrap()).unwrap();

                        let mut item = Item {
                            title: title,
                            icon: Icon {
                                width: usize::try_from(icon.0).unwrap(),
                                height: usize::try_from(icon.1).unwrap(),
                                path: temp_dir.to_str().unwrap().to_string(),
                            },
                        };

                        s2.send((args.service.to_string(), Some(item.clone()))).await.unwrap();

                        //signals.scan(None, |state, signal| async move {
                        //    //proxy.get_property("");
                        //    dbg!(signal);
                        //    Some(state)
                        //});
                        signals
                            .for_each(|t| async move {
//                                dbg!(t);
                            })
                            .await;
                        Ok::<(), zbus::Error>(())
                    },
                    async {
                        while let Some(name) = owner_change.next().await {
                            if name == None {
                                break;
                            }
                        }
                        s.send(args.service.to_string()).await.unwrap();
                        Ok::<(), zbus::Error>(())
                    }
                )
                .unwrap();
            }
        });
    let mut items = HashMap::new();

    try_join!(
        async {
            while let Some((service, item)) = r2.recv().await.ok() {
                if let Some(v) = item{
                    items.insert(service, v);
                }else {
                    items.remove(&service);
                }
                let j = json!(items.values().collect::<Vec<&Item>>());
                println!("{}",serde_json::to_string(&j).unwrap());
            }
            Ok::<(), zbus::Error>(())
        },
        async {
            task1.await;
            Ok::<(), zbus::Error>(())
        },
        async {
            while let Some(service) = r.recv().await.ok() {
                c1.call_method(
                    Some("org.kde.StatusNotifierWatcher"),
                    "/StatusNotifierWatcher",
                    Some("org.kde.StatusNotifierWatcher"),
                    "UnregisterStatusNotifierItem",
                    &(service),
                )
                .await
                .unwrap();
                s2.send((service, None)).await.unwrap();
            }
            Ok::<(), zbus::Error>(())
        }
    )?;
    loop {
        std::thread::park();
    }
}
