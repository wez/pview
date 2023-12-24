/// Show diagnostic information about a shade
#[derive(clap::Parser, Debug)]
pub struct InspectShadeCommand {
    /// The name or id of the shade to inspect.
    /// Names will be compared ignoring case.
    name: String,
}

impl InspectShadeCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;

        let shade = hub.shade_by_name(&self.name).await?;

        println!("{shade:#?}");
        Ok(())
    }
}
