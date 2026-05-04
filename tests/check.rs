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
        .stdout(predicate::str::contains("change:           removed"))
        .stdout(predicate::str::contains(
            "shopware source:  src/Core/Checkout/Cart/CartService.php:7",
        ))
        .stdout(predicate::str::contains(
            "public function recalculate(): void",
        ))
        .stdout(predicate::str::contains("affected plugins: 1"))
        .stdout(predicate::str::contains("swag/cart"))
        .stdout(predicate::str::contains(
            "https://github.com/shopware/store-plugin-mirror/blob/main/plugins/shopware6/plugin/SwagCart/src/CartSubscriber.php#L",
        ));

    Command::cargo_bin("sw-impact")
        .expect("binary exists")
        .args([
            "check",
            "--shopware",
            shopware.to_str().expect("utf-8 shopware path"),
            "--index",
            index.to_str().expect("utf-8 index path"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Base:"))
        .stdout(predicate::str::contains("HEAD"))
        .stdout(predicate::str::contains(
            "php:method:Shopware\\Core\\Checkout\\Cart\\CartService::recalculate",
        ))
        .stdout(predicate::str::contains("affected plugins: 1"));
}

#[test]
fn check_ignores_shopware_xml_changes() {
    let temp = tempdir().expect("tempdir");
    let plugins = temp.path().join("plugins");
    let plugin = plugins.join("SwagLanguage");
    let plugin_config = plugin.join("src/Resources/config");
    let index = temp.path().join("plugins.sqlite");
    let shopware = temp.path().join("shopware");
    let shopware_config = shopware.join("src/Core/Content/DependencyInjection");
    let mail_template = shopware_config.join("mail_template.xml");

    fs::create_dir_all(&plugin_config).expect("create plugin config");
    fs::write(plugin.join("composer.json"), r#"{"name":"swag/language"}"#).expect("write composer");
    fs::write(
        plugin_config.join("services.xml"),
        r#"<container>
  <services>
    <service id="swag.language.consumer">
      <argument type="service" id="language.repository"/>
    </service>
  </services>
</container>
"#,
    )
    .expect("write plugin services");

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

    fs::create_dir_all(&shopware_config).expect("create shopware config");
    fs::write(
        &mail_template,
        r#"<container>
  <services>
    <service id="shopware.mail_template.service"/>
  </services>
</container>
"#,
    )
    .expect("write base shopware xml");

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
        &mail_template,
        r#"<container>
  <services>
    <service id="shopware.mail_template.service">
      <argument type="service" id="language.repository"/>
    </service>
  </services>
</container>
"#,
    )
    .expect("add service reference in shopware xml");

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
        .stdout(predicate::str::contains("Changed surfaces:"))
        .stdout(predicate::str::contains(
            "No changed Shopware definition surfaces found.",
        ))
        .stdout(predicate::str::contains("service:id:language.repository").not());
}

#[test]
fn check_removed_admin_component_ignores_unrelated_js_usage_surfaces() {
    let temp = tempdir().expect("tempdir");
    let plugins = temp.path().join("plugins");
    let plugin = plugins.join("SwagProductExtension");
    let plugin_admin = plugin.join("src/Resources/app/administration/src");
    let index = temp.path().join("plugins.sqlite");
    let shopware = temp.path().join("shopware");
    let shopware_admin =
        shopware.join("src/Administration/Resources/app/administration/src/module/sw-product");
    let module_index = shopware_admin.join("index.js");

    fs::create_dir_all(&plugin_admin).expect("create plugin admin source");
    fs::write(
        plugin.join("composer.json"),
        r#"{"name":"swag/product-extension"}"#,
    )
    .expect("write composer");
    fs::write(
        plugin_admin.join("main.js"),
        r#"
Shopware.Component.override('sw-product-detail', {});
Shopware.Component.extend('swag-product-detail', 'sw-product-detail', {});
"#,
    )
    .expect("write plugin admin source");

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

    fs::create_dir_all(&shopware_admin).expect("create shopware admin source");
    fs::write(
        &module_index,
        r#"
Shopware.Component.register('sw-product-detail', () => import('./page/sw-product-detail'));

const repo = repositoryFactory.create('product');
"#,
    )
    .expect("write base shopware module");

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

    fs::remove_file(&module_index).expect("remove shopware component registration");

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
        .stdout(predicate::str::contains("Changed surfaces:"))
        .stdout(predicate::str::contains(
            "admin:component:sw-product-detail",
        ))
        .stdout(predicate::str::contains(
            "surface kind:     admin-component",
        ))
        .stdout(predicate::str::contains("change:           removed"))
        .stdout(predicate::str::contains("affected plugins: 1"))
        .stdout(predicate::str::contains("swag/product-extension"))
        .stdout(predicate::str::contains("dal:entity:product").not());
}

#[test]
fn check_reports_removed_twig_block_impact() {
    let temp = tempdir().expect("tempdir");
    let plugins = temp.path().join("plugins");
    let plugin = plugins.join("SwagAdminBlock");
    let plugin_views = plugin.join("src/Resources/views/administration");
    let index = temp.path().join("plugins.sqlite");
    let shopware = temp.path().join("shopware");
    let shopware_views = shopware
        .join("src/Administration/Resources/views/administration/module/sw-product/page/sw-product-detail");
    let template = shopware_views.join("sw-product-detail.html.twig");

    fs::create_dir_all(&plugin_views).expect("create plugin views");
    fs::write(
        plugin.join("composer.json"),
        r#"{"name":"swag/admin-block"}"#,
    )
    .expect("write composer");
    fs::write(
        plugin_views.join("sw-product-detail.html.twig"),
        r#"
{% block sw_product_detail_content_tabs_reviews %}
    {{ parent() }}
{% endblock %}
"#,
    )
    .expect("write plugin twig");

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

    fs::create_dir_all(&shopware_views).expect("create shopware views");
    fs::write(
        &template,
        r#"
{% block sw_product_detail %}
    {% block sw_product_detail_content_tabs_reviews %}
        <sw-card />
    {% endblock %}
{% endblock %}
"#,
    )
    .expect("write base shopware twig");

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
        &template,
        r#"
{% block sw_product_detail %}
    <sw-card />
{% endblock %}
"#,
    )
    .expect("remove shopware twig block");

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
            "twig:block:sw_product_detail_content_tabs_reviews",
        ))
        .stdout(predicate::str::contains("surface kind:     twig-block"))
        .stdout(predicate::str::contains("change:           removed"))
        .stdout(predicate::str::contains("affected plugins: 1"))
        .stdout(predicate::str::contains("swag/admin-block"));
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
