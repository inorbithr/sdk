use std::path::Path;

use serde_json::Value;

use crate::Env;
use crate::cli::Global;
use crate::context::session;
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn pull(g: &Global, env: &Env, output: &Path, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    let resp = s
        .api
        .send(reqwest::Method::GET, "/v1/openapi.json", &[], None)
        .await?;
    let doc: Value = resp.json()?;
    let operations = doc.get("paths").and_then(Value::as_object).map_or(0, |p| {
        p.values()
            .filter_map(Value::as_object)
            .map(serde_json::Map::len)
            .sum::<usize>()
    });
    if output.as_os_str() == "-" {
        Out::raw(&resp.body);
        return Ok(());
    }
    std::fs::write(output, &resp.body)
        .map_err(|e| Error::Failed(format!("cannot write {}: {e}", output.display())))?;
    if out.json {
        Out::print_json(&serde_json::json!({ "file": output, "operations": operations }));
    } else {
        Out::note(&format!(
            "Wrote {} ({operations} operations in the document for this credential's plan).",
            output.display()
        ));
    }
    Ok(())
}
