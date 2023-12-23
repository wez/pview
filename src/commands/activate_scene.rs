#[derive(clap::Parser, Debug)]
pub struct ActivateSceneCommand {
    /// The name or id of the shade to inspect.
    /// Names will be compared ignoring case.
    name: String,
}

impl ActivateSceneCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;

        let scene = hub.scene_by_name(&self.name).await?;
        let shades = hub.activate_scene(scene.id).await?;

        println!("{shades:#?}");
        Ok(())
    }
}
