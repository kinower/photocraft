//! Filter > Plug-ins > GenFill-K. Fills the selection with Codex ImageGen (`/imagen`) from its
//! own window and its own job: it needs neither GenGate's window, engine nor queue. Its layers
//! are Ai Fill galleries, so variants are still chosen in Properties.
use crate::PhotocraftApp;
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::genstudio_ai;
use serde_json::{Value, json};

const LABEL: &str = "GenFill-K";

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub prompt: String,
    /// The last layer made here and the selection it came from: generating again from that
    /// selection adds a variant to that layer instead of making a new one.
    source: Option<(DocId, LayerId, photocraft_raster::Surface)>,
}

pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "genfillk.open" {
        return None;
    }
    let open = match params.get("open") {
        None => true,
        Some(Value::Bool(open)) => *open,
        Some(_) => return Some(Err("GenFill-K open must be a boolean".into())),
    };
    app.genfill_k.open = open;
    Some(Ok(json!({"window":"genFillK","open":open})))
}

/// The active GenFill-K layer, when generating should add a variant to it: no selection, or the
/// same selection the layer was made from.
fn reusable(app: &PhotocraftApp) -> Option<LayerId> {
    let st = app.session.active()?;
    let layer = st.doc.layer(st.active_layer?)?;
    if !genstudio_ai::gallery(layer).is_ok_and(|g| g.engine.is_some()) {
        return None;
    }
    let from_here = |selection: &photocraft_raster::Surface| {
        app.genfill_k.source.as_ref().is_some_and(|(d, l, s)| *d == st.doc.id && *l == layer.id && genstudio_ai::shared_mask(s, selection))
    };
    st.doc.selection.as_ref().is_none_or(from_here).then_some(layer.id)
}

fn generate(app: &mut PhotocraftApp) -> Result<Value, String> {
    let text = app.genfill_k.prompt.clone();
    let layer = match reusable(app) {
        Some(layer) => layer.0,
        None => {
            let selection = app.session.active().and_then(|st| st.doc.selection.clone());
            let layer = app.run("genfillk.create", json!({"prompt":text}))?["layer"].as_u64().ok_or("GenFill-K layer was not created")?;
            if let (Some(doc), Some(selection)) = (app.session.active().map(|st| st.doc.id), selection) {
                app.genfill_k.source = Some((doc, LayerId(layer), selection));
            }
            layer
        }
    };
    let result = app.run("genfillk.generate", json!({"layer":layer,"prompt":text}))?;
    app.ui.mask_target = false;
    app.ui.properties_layer = Some(layer);
    app.ui.panels.properties = true;
    app.ui.dock.collapsed.retain(|group| *group != crate::dock::Group::Properties);
    Ok(result)
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.genfill_k.open {
        return;
    }
    let doc = app.session.active().map(|st| st.doc.id);
    let job = app.session.jobs().into_iter().find(|job| job.state == "running" && job.label == LABEL);
    let busy = doc.is_some_and(|doc| app.session.job_on(doc).is_some());
    let target = reusable(app);
    let can = !busy && (target.is_some() || app.session.is_enabled("genfillk.create"));
    let mut open = true;
    let mut clicked = false;
    egui::Window::new(LABEL).id(egui::Id::unique("genfill-k-window")).open(&mut open).resizable(false).collapsible(false).default_width(340.0).show(
        ctx,
        |ui| {
            ui.small("Codex ImageGen (/imagen). Fills the selection grown 10 px and feathered 4 px.");
            ui.add_space(4.0);
            ui.add_sized(
                egui::vec2(ui.available_width(), 72.0),
                egui::TextEdit::multiline(&mut app.genfill_k.prompt)
                    .char_limit(6000)
                    .hint_text("What goes in the selection (empty removes what is there)")
                    .id_salt("genfill-k-prompt"),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = if target.is_some() { "Add Variant" } else { "Generate" };
                clicked = ui.add_enabled(can, egui::Button::new(label)).clicked();
                if let Some(job) = &job {
                    ui.add(egui::ProgressBar::new(job.progress).text(job.message.as_str()).desired_width(ui.available_width()));
                } else if busy {
                    ui.weak("The document is busy");
                } else if !can {
                    ui.weak("Make a selection first");
                }
            });
        },
    );
    app.genfill_k.open = open;
    if clicked && let Err(e) = generate(app) {
        crate::notices::error(app, e);
    }
    if job.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genfill_k_sits_in_filter_plug_ins_and_opens_its_own_window() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(crate::menus::is_enabled(&app, "genfillk.open"));
        let item = crate::menus::menu_items(&app).into_iter().find(|i| i.id == "genfillk.open").unwrap();
        assert_eq!(item.path, ["Filter", "Plug-ins"]);
        crate::menus::invoke(&mut app, &ctx, "genfillk.open", json!({})).unwrap();
        assert!(app.genfill_k.open);
        assert!(!app.ui.gen_studio.open, "GenFill-K does not open GenGate");
        crate::menus::invoke(&mut app, &ctx, "genfillk.open", json!({"open":false})).unwrap();
        assert!(!app.genfill_k.open);
    }

    #[test]
    fn genfill_k_adds_variants_to_its_layer_only_for_the_same_selection() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":128,"height":96})).unwrap();
        app.run("select.rect", json!({"x":20,"y":20,"width":30,"height":30})).unwrap();
        assert_eq!(reusable(&app), None);
        let selection = app.session.active().and_then(|st| st.doc.selection.clone()).unwrap();
        let layer = LayerId(app.run("genfillk.create", json!({"prompt":"An olive tree"})).unwrap()["layer"].as_u64().unwrap());
        app.genfill_k.source = Some((app.session.active().unwrap().doc.id, layer, selection));
        assert_eq!(reusable(&app), Some(layer));
        app.run("select.rect", json!({"x":60,"y":20,"width":30,"height":30})).unwrap();
        assert_eq!(reusable(&app), None, "another selection makes another layer");
        app.run("select.deselect", json!({})).unwrap();
        assert_eq!(reusable(&app), Some(layer));
    }
}
