/// Show diagnostic information for the hub
#[derive(clap::Parser, Debug)]
pub struct HubInfoCommand {}
impl HubInfoCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;
        let user_data = hub.get_user_data().await?;
        println!("{user_data:#?}");
        Ok(())
    }
}
