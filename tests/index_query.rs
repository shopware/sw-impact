use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
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
        .success()
        .stderr(predicate::str::contains("Indexed 1 plugins"))
        .stderr(predicate::str::contains(" in "));

    let connection = Connection::open(&index).expect("open generated index");
    let plugin_path: String = connection
        .query_row(
            "select path from plugin where name = 'swag/cart'",
            [],
            |row| row.get(0),
        )
        .expect("plugin path");
    let corpus_root: String = connection
        .query_row(
            "select value from metadata where key = 'corpus_root'",
            [],
            |row| row.get(0),
        )
        .expect("corpus root metadata");

    assert_eq!(plugin_path, "SwagCart");
    assert_eq!(corpus_root, "plugins");
    assert!(!plugin_path.contains(temp.path().to_string_lossy().as_ref()));
    assert!(!corpus_root.contains(temp.path().to_string_lossy().as_ref()));

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
        .stdout(predicate::str::contains("Shopware Impact Query"))
        .stdout(predicate::str::contains("Affected plugins:"))
        .stdout(predicate::str::contains("1 / 1 (100.0%)"))
        .stdout(predicate::str::contains("swag/cart"))
        .stdout(predicate::str::contains("src/CartSubscriber.php"))
        .stdout(predicate::str::contains(
            "https://github.com/shopware/store-plugin-mirror/blob/main/plugins/shopware6/plugin/SwagCart/src/CartSubscriber.php#L",
        ));

    Command::cargo_bin("sw-impact")
        .expect("binary exists")
        .args([
            "query",
            "--index",
            index.to_str().expect("utf-8 index path"),
            "Cart",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Query:"))
        .stdout(predicate::str::contains("Cart"))
        .stdout(predicate::str::contains("Matching surfaces:"))
        .stdout(predicate::str::contains(
            "php:class:Shopware\\Core\\Checkout\\Cart\\CartService",
        ))
        .stdout(predicate::str::contains("src/CartSubscriber.php"));

    Command::cargo_bin("sw-impact")
        .expect("binary exists")
        .args([
            "query",
            "--index",
            index.to_str().expect("utf-8 index path"),
            "php:class:*CartService",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Query:"))
        .stdout(predicate::str::contains("php:class:*CartService"))
        .stdout(predicate::str::contains(
            "php:class:Shopware\\Core\\Checkout\\Cart\\CartService",
        ));
}
