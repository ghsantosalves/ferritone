//! The Preferences assistant that helps make `ferritone` open the player in a browser.

use super::*;

/// Which instructions a browser needs.
fn browser_kind(app: &gio::AppInfo) -> &'static str {
    let id = app.id().map(|id| id.to_lowercase()).unwrap_or_default();
    let name = app.name().to_lowercase();
    let either = |word: &str| id.contains(word) || name.contains(word);
    if either("firefox") || either("librewolf") || either("waterfox") || either("zen") {
        "firefox"
    } else if either("brave") {
        "brave"
    } else if either("edge") {
        "edge"
    } else if either("vivaldi") {
        "vivaldi"
    } else if either("chrom") || either("opera") {
        "chromium"
    } else {
        "other"
    }
}

impl Player {
    pub(crate) fn browser_shortcut_preferences(self: &Rc<Self>, form: &gtk::Box) {
        let title = gtk::Label::new(Some(self.text("Browser shortcut", "Atalho no navegador")));
        title.add_css_class("section-title");
        title.set_xalign(0.0);
        form.append(&title);
        let description = gtk::Label::new(Some(self.text(
            "Open the player by typing ferritone in your browser while Ferritone is open.",
            "Abra o player digitando ferritone no navegador enquanto o Ferritone estiver aberto.",
        )));
        description.set_xalign(0.0);
        description.set_wrap(true);
        form.append(&description);
        let install = button(
            self.text("Install browser shortcut", "Instalar atalho no navegador"),
            self,
            |player| player.show_browser_shortcut(),
        );
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("page-actions");
        actions.append(&install);
        form.append(&actions);
    }

    fn show_browser_shortcut(self: &Rc<Self>) {
        let Some(home) = self.remote_local_url() else {
            self.message(self.text(
                "The player page is not available right now.",
                "A página do player não está disponível agora.",
            ));
            return;
        };
        let window = gtk::Window::builder()
            .title(self.text("Browser shortcut", "Atalho no navegador"))
            .transient_for(&self.window)
            .modal(true)
            .resizable(false)
            .default_width(340)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        let intro = gtk::Label::new(Some(self.text(
            "Choose the browser to set up. Ferritone opens it with the steps and copies the player's address.",
            "Escolha o navegador para configurar. O Ferritone abre o navegador com o passo a passo e copia o endereço do player.",
        )));
        intro.set_wrap(true);
        intro.set_max_width_chars(40);
        intro.set_xalign(0.0);
        intro.set_margin_bottom(6);
        content.append(&intro);

        let mut seen = HashSet::new();
        let browsers: Vec<gio::AppInfo> = gio::AppInfo::all_for_type("x-scheme-handler/https")
            .into_iter()
            .filter(|app| app.should_show())
            // Other apps also open https links, such as chat apps; only list real browsers.
            .filter(|app| {
                app.downcast_ref::<gio::DesktopAppInfo>()
                    .and_then(|desktop| desktop.categories())
                    .is_some_and(|categories| {
                        categories
                            .split(';')
                            .any(|category| category == "WebBrowser")
                    })
            })
            .filter(|app| seen.insert(app.id().map(|id| id.to_string()).unwrap_or_default()))
            .collect();
        if browsers.is_empty() {
            let none = gtk::Label::new(Some(self.text(
                "No web browser was found.",
                "Nenhum navegador foi encontrado.",
            )));
            none.add_css_class("dim-label");
            content.append(&none);
        }
        for app in browsers {
            let kind = browser_kind(&app);
            let setup = format!("{home}setup?browser={kind}");
            let close = window.downgrade();
            let home = home.clone();
            let launcher = app.clone();
            let item = button("", self, move |player| {
                player.window.clipboard().set_text(&home);
                let context = gtk::prelude::WidgetExt::display(&player.window).app_launch_context();
                if launcher.launch_uris(&[&setup], Some(&context)).is_err() {
                    player.message(player.text(
                        "The browser could not be opened.",
                        "Não foi possível abrir o navegador.",
                    ));
                }
                if let Some(window) = close.upgrade() {
                    window.close();
                }
            });
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let icon = app.icon().map_or_else(
                || gtk::Image::from_icon_name("web-browser-symbolic"),
                |icon| gtk::Image::from_gicon(&icon),
            );
            icon.set_pixel_size(24);
            line.append(&icon);
            let name = gtk::Label::new(Some(&app.display_name()));
            name.set_xalign(0.0);
            line.append(&name);
            item.set_child(Some(&line));
            content.append(&item);
        }
        window.set_child(Some(&content));
        window.present();
    }
}
