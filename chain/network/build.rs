use sha2::Digest;
use std::path::Path;

fn content_prefix(content: &str) -> String {
    format!(
        "/// this file sha256 = {}\n",
        base64::encode(&sha2::Sha256::digest(content.as_bytes()))
    )
}

fn is_fresh(want_prefix: &str, generated: &Path) -> bool {
    let generated = if let Ok(g) = std::fs::read_to_string(generated) {
        g
    } else {
        return false;
    };
    let generated = if let Some(g) = generated.strip_prefix(want_prefix) {
        g
    } else {
        return false;
    };
    let (prefix, content) = if let Some(pc) = generated.split_once('\n') {
        pc
    } else {
        return false;
    };
    return format!("{prefix}\n") == content_prefix(content);
}

#[allow(unreachable_code)]
fn main() -> anyhow::Result<()> {
    // Generate code from proto, whenever proto files change.
    // The generated code is checked into the repo, so that
    // building near node doesn't have a dependency on protoc
    // being installed locally.
    // Ideally, during "cargo build" protoc should be rather build from source
    // or a pre-compiled binary should be downloaded.
    // TODO: generalize to an arbitrary number of protos.

    // TODO: this is disgusting, generated code shouldn't be checked into git.
    let proto_path = Path::new("src/network_protocol/network.proto");
    let generated_path = Path::new("src/network_protocol/generated/network.rs");

    let want_hash = sha2::Sha256::digest(std::fs::read_to_string(proto_path)?.as_ref());
    let mut want_prefix = String::new();
    want_prefix += "/// This is an autogenerated file. DO NOT EDIT.\n";
    want_prefix += &format!("/// proto file sha256 = {}\n", base64::encode(&want_hash));

    if is_fresh(&want_prefix, generated_path) {
        return Ok(());
    }

    eprintln!("proto file changed, need to regenerate the code");
    eprintln!("This requires protoc to be installed and available in $PATH");
    // prost-build checks the presence of protoc in its build.rs and panics
    // if it is not found and it is unable to compile it.
    // To avoid that I had to make the build dependency on prost-build optional
    // add the conditional compilation here.
    #[cfg(feature = "prost-build")]
    prost_build::Config::new()
        .out_dir("src/network_protocol/generated")
        .compile_protos(&[proto_path], &["src/"])?;
    #[cfg(not(feature = "prost-build"))]
    anyhow::bail!("To regenerate the code, run 'cargo build --features prost-build'");
    let content = std::fs::read_to_string(generated_path)?;
    let mut generated = want_prefix;
    generated += &content_prefix(&content);
    generated += &content;
    std::fs::write(generated_path, generated)?;
    Ok(())
}