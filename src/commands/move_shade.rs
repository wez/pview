use crate::api_types::ShadeUpdateMotion;

#[derive(clap::Args, Debug)]
#[group(required = true)]
struct TargetPosition {
    #[arg(long, conflicts_with = "percent")]
    motion: Option<ShadeUpdateMotion>,
    #[arg(long, group = "position")]
    percent: Option<u8>,
}

#[derive(clap::Parser, Debug)]
pub struct MoveShadeCommand {
    /// The name or id of the shade to open.
    /// Names will be compared ignoring case.
    name: String,
    #[command(flatten)]
    target_position: TargetPosition,
}

impl MoveShadeCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        let hub = args.hub().await?;

        let shade = hub.shade_by_name(&self.name).await?;

        let shade = if let Some(motion) = self.target_position.motion {
            hub.move_shade(shade.id, motion).await?
        } else if let Some(percent) = self.target_position.percent {
            let absolute = ((u16::max_value() as u32) * (percent as u32) / 100u32) as u16;

            let mut position = shade.positions.clone().ok_or_else(|| {
                anyhow::anyhow!("shade has no existing position information! {shade:#?}")
            })?;
            if shade.is_primary() {
                position.position_1 = absolute;
            } else {
                position.position_2.replace(absolute);
            }

            hub.change_shade_position(shade.id, position).await?
        } else {
            anyhow::bail!("One of --motion or --percent is required");
        };

        println!("{shade:#?}");
        Ok(())
    }
}
