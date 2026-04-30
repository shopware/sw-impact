use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn check_reports_indexed_plugin_impact_for_removed_method() {
    let temp = tempdir().expect("tempdir");
    let plugins = temp.path().join("plugins");
    let plugin = plugins.join("SwagCart");
    let plugin_source = plugin.join("src");
    let index = temp.path().join("plugins.sqlite");
    let shopware = temp.path().join("shopware");
    let shopware_source = shopware.join("src/Core/Checkout/Cart");
    let cart_service = shopware_source.join("CartService.php");

    fs::create_dir_all(&plugin_source).expect("create plugin source");
    fs::write(plugin.join("composer.json"), r#"{"name":"swag/cart"}"#).expect("write composer");
    fs::write(
        plugin_source.join("CartSubscriber.php"),
        r#"<?php

namespace Swag\Cart;

use Shopware\Core\Checkout\Cart\CartService;

final class CartSubscriber
{
    public function run(): void
    {
        CartService::recalculate();
    }
}
"#,
    )
    .expect("write plugin source");

    Command::cargo_bin("sw-impact")
        .expect("binary exists")
        .args([
            "index",
            "--plugins",
            plugins.to_str().expect("utf-8 plugins path"),
            "--out",
            index.to_str().expect("utf-8 index path"),
        ])
        .assert()
        .success();

    fs::create_dir_all(&shopware_source).expect("create shopware source");
    fs::write(
        &cart_service,
        r#"<?php

namespace Shopware\Core\Checkout\Cart;

class CartService
{
    public function recalculate(): void
    {
    }
}
"#,
    )
    .expect("write base shopware source");

    git(&shopware, ["init", "--initial-branch=main"]).expect("git init");
    git(&shopware, ["add", "."]).expect("git add");
    git(
        &shopware,
        [
            "-c",
            "user.name=Test User",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "base",
        ],
    )
    .expect("git commit");

    fs::write(
        &cart_service,
        r#"<?php

namespace Shopware\Core\Checkout\Cart;

class CartService
{
}
"#,
    )
    .expect("remove method from current worktree");

    Command::cargo_bin("sw-impact")
        .expect("binary exists")
        .args([
            "check",
            "--shopware",
            shopware.to_str().expect("utf-8 shopware path"),
            "--base",
            "HEAD",
            "--index",
            index.to_str().expect("utf-8 index path"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "php:method:Shopware\\Core\\Checkout\\Cart\\CartService::recalculate",
        ))
        .stdout(predicate::str::contains("change: removed"))
        .stdout(predicate::str::contains("affected plugins: 1"))
        .stdout(predicate::str::contains("swag/cart"));
}

fn git<const N: usize>(repo: &Path, args: [&str; N]) -> std::io::Result<()> {
    let output = StdCommand::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    assert!(
        output.status.success(),
        "git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
