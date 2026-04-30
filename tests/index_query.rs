use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn index_then_query_returns_plugin_evidence() {
    let temp = tempdir().expect("tempdir");
    let plugins = temp.path().join("plugins");
    let plugin = plugins.join("SwagCart");
    let source = plugin.join("src");
    let index = temp.path().join("plugins.sqlite");

    fs::create_dir_all(&source).expect("create plugin source");
    fs::write(
        plugin.join("composer.json"),
        r#"{"name":"swag/cart","version":"1.0.0"}"#,
    )
    .expect("write composer");
    fs::write(
        source.join("CartSubscriber.php"),
        r#"<?php

namespace Swag\Cart;

use Shopware\Core\Checkout\Cart\CartService;

final class CartSubscriber
{
    public function __construct(private CartService $cartService)
    {
    }
}
"#,
    )
    .expect("write php source");
    fs::write(
        source.join("InvalidUtf8.php"),
        b"<?php\n// Shopware marker with invalid byte \xFF\nuse Shopware\\Core\\Framework\\Context;\n",
    )
    .expect("write invalid utf-8 php source");

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

    Command::cargo_bin("sw-impact")
        .expect("binary exists")
        .args([
            "query",
            "--index",
            index.to_str().expect("utf-8 index path"),
            "php:class:Shopware\\Core\\Checkout\\Cart\\CartService",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Affected plugins: 1"))
        .stdout(predicate::str::contains("swag/cart"))
        .stdout(predicate::str::contains("src/CartSubscriber.php"));
}
