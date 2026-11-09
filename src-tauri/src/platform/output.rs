use super::*;

impl Platform {
    fn output_helper(&self) -> PathBuf {
        self.resources.join("native/bin/mac-output")
    }
    fn ensure_native_output_platform(&self) -> Result<()> {
        ensure!(
            cfg!(all(target_os = "macos", target_arch = "aarch64")),
            "原生自动选源当前仅支持 Apple Silicon Mac"
        );
        Ok(())
    }
    pub async fn output_list(&self) -> Result<Value> {
        match platform() {
            "darwin" => Ok(serde_json::from_str(
                &self.run(self.output_helper(), &["list".into()]).await?,
            )?),
            "win32" => self.win("output-options", serde_json::json!({})).await,
            _ => Ok(serde_json::json!([])),
        }
    }
    pub async fn capture_output(&self, output: &str) -> Result<Value> {
        match platform() {
            "darwin" => {
                self.ensure_native_output_platform()?;
                uuid::Uuid::parse_str(output).context("请选择外接屏 UUID")?;
                Ok(serde_json::from_str(
                    &self
                        .run(self.output_helper(), &["capture".into(), output.into()])
                        .await?,
                )?)
            }
            "win32" => {
                self.win("output-capture", serde_json::json!({"monitorId":output}))
                    .await
            }
            _ => bail!("当前系统暂不支持自动选源兜底"),
        }
    }
    pub async fn disable_output(&self, _output: &str, baseline: &Value) -> Result<()> {
        match platform() {
            "darwin" => {
                self.ensure_native_output_platform()?;
                self.run(
                    self.output_helper(),
                    &["disable".into(), baseline.to_string()],
                )
                .await?;
            }
            "win32" => {
                self.win("output-disable", serde_json::json!({"topology":baseline}))
                    .await?;
            }
            _ => bail!("当前系统暂不支持自动选源兜底"),
        }
        Ok(())
    }
    pub async fn restore_output(&self, _output: &str, baseline: &Value) -> Result<()> {
        match platform() {
            "darwin" => {
                self.ensure_native_output_platform()?;
                self.run(
                    self.output_helper(),
                    &["restore".into(), baseline.to_string()],
                )
                .await?;
            }
            "win32" => {
                self.win("output-restore", serde_json::json!({"topology":baseline}))
                    .await?;
            }
            _ => bail!("当前系统暂不支持自动选源兜底"),
        }
        Ok(())
    }
}
