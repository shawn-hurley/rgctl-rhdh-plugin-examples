use super::{connections, index, model::*, store};
use std::fs;

fn request(name: &str) -> Onboard {
    Onboard {
        entity_ref: format!("component:default/{name}"),
        source: Source {
            repository_url: "https://github.com/example/java-service.git".into(),
            revision: "HEAD".into(),
            sub_path: String::new(),
            include_maven_dependencies: false,
        },
        provides_apis: vec![],
        consumes_apis: vec![],
    }
}
fn build(name: &str, source: &str) -> (tempfile::TempDir, Config, Manifest) {
    build_files(name, &[("Application.java", source)])
}
fn build_files(name: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, Config, Manifest) {
    let temp = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: temp.path().join("data"),
        allowed_repository_hosts: vec!["github.com".into()],
        allowed_source_roots: vec![],
        maven_repository: None,
        queue_capacity: 8,
    };
    let source_dir = temp.path().join("source/src/main/java");
    fs::create_dir_all(&source_dir).unwrap();
    for (file, source) in files {
        let file = source_dir.join(file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let dir = store::job_dir(&config, &id).unwrap();
    fs::create_dir_all(&dir).unwrap();
    let manifest = index::index(
        &request(name),
        &id,
        &dir,
        &temp.path().join("source"),
        "abc123".into(),
        None,
    )
    .unwrap();
    (temp, config, manifest)
}

#[test]
fn spring_annotations_compose_class_and_method_routes() {
    let (_, _, m) = build(
        "orders",
        r#"
package example;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.GetMapping;
@RequestMapping("/api")
public class Orders {
    @GetMapping("/orders/{id}")
    public String getOne(String id) { return id; }
}
"#,
    );
    assert_eq!(m.operations.len(), 1, "{:?}", m.operations);
    assert_eq!(m.operations[0].path.as_deref(), Some("/api/orders/{id}"));
    assert_eq!(m.operations[0].methods, vec!["GET"]);
    assert_eq!(
        m.operations[0].symbol.qualified_name,
        "example.Orders.getOne"
    );
}

#[test]
fn jaxrs_and_declarative_clients_use_native_annotations() {
    let (_, _, m) = build(
        "jax",
        r#"
package example;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.GET;
@Path("/quotes")
public class Quotes {
    @GET @Path("/random")
    public String random() { return "a"; }
}
"#,
    );
    assert!(
        m.operations
            .iter()
            .any(|o| o.direction == "provides" && o.path.as_deref() == Some("/quotes/random"))
    );
    let (_, _, m) = build(
        "feign",
        r#"
package example;
import org.springframework.cloud.openfeign.FeignClient;
import org.springframework.web.bind.annotation.GetMapping;
@FeignClient(name="quotes")
public interface QuotesClient {
    @GetMapping("/quotes/random")
    String random();
}
"#,
    );
    assert!(
        m.operations
            .iter()
            .any(|o| o.direction == "consumes" && o.path.as_deref() == Some("/quotes/random")),
        "{:?}",
        m.operations
    );
}

#[test]
fn fluent_client_in_lambda_is_discovered_without_compilation() {
    let (_, _, m) = build(
        "client",
        r#"
package example;
import org.springframework.web.client.RestClient;
public class Client {
    public Runnable run(RestClient client) {
        return () -> client.get().uri("/api/random").retrieve().body(String.class);
    }
}
"#,
    );
    assert!(
        m.operations.iter().any(|o| o.direction == "consumes"
            && o.methods == ["GET"]
            && o.path.as_deref() == Some("/api/random")),
        "{:?}",
        m.operations
    );
}

#[test]
fn dynamic_urls_are_retained_as_unresolved_instead_of_fabricated_paths() {
    let (_, _, m) = build(
        "client",
        r#"
package example;
import org.springframework.web.client.RestTemplate;
public class Client {
    public String lookup(RestTemplate client, String url) { return client.getForObject(url, String.class); }
}
"#,
    );
    assert!(
        m.operations
            .iter()
            .any(|o| o.direction == "consumes" && o.path.is_none())
    );
    assert!(
        m.diagnostics
            .iter()
            .any(|d| d["code"] == "UNRESOLVED_HTTP_URL")
    );
}

#[test]
fn similarly_named_custom_annotations_are_not_http_routes() {
    let (_, _, m) = build(
        "custom",
        r#"
package example;
@interface GetMapping { String value(); }
public class Custom {
    @GetMapping("/api/random")
    public String get() { return "nothing"; }
}
"#,
    );
    assert!(m.operations.is_empty());
}

#[test]
fn snapshots_survive_reopening_and_detect_corruption() {
    let (_temp, config, m) = build(
        "plain",
        "package example; public class Plain { public String call() { return helper(); } public String helper() { return \"ok\"; } }",
    );
    store::write(
        &config
            .data_dir
            .join("applications")
            .join(format!("{}.json", m.application_key)),
        &m,
    )
    .unwrap();
    let reloaded = store::current(&config, &m.application_key).unwrap();
    let graph = index::snapshot(&config, &reloaded).unwrap();
    assert!(graph.node_count() > 0 && graph.edge_count() > 0);
    drop(graph);
    fs::write(
        store::job_dir(&config, &m.graph_version)
            .unwrap()
            .join("graph.snapshot.bin"),
        b"broken",
    )
    .unwrap();
    assert!(index::snapshot(&config, &m).is_err());
}

#[test]
fn catalog_identity_gates_operation_matching_and_exact_routes_win() {
    let (_, _, mut provider) = build(
        "provider",
        r#"
package example;
import org.springframework.web.bind.annotation.GetMapping;
public class Routes {
    @GetMapping("/api/random") public String random() { return "a"; }
    @GetMapping("/api/{id}") public String one(String id) { return id; }
}
"#,
    );
    let (_, _, mut consumer) = build(
        "consumer",
        r#"
package example;
import org.springframework.web.client.RestTemplate;
public class Client { public String run(RestTemplate c) { return c.getForObject("http://localhost:8080/api/random",String.class); } }
"#,
    );
    assert!(connections::connect(&[provider.clone(), consumer.clone()]).is_empty());
    provider.provides_apis = vec!["api:default/quotes".into()];
    consumer.consumes_apis = vec!["api:default/quotes".into()];
    let links = connections::connect(&[provider.clone(), consumer.clone()]);
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0]["operationMatches"].as_array().unwrap().len(),
        1,
        "{links:?}"
    );
    assert_eq!(
        links[0]["operationMatches"][0]["callee"]["symbol"]["qualifiedName"],
        "example.Routes.random"
    );
    consumer.consumes_apis = vec!["api:other/quotes".into()];
    assert!(connections::connect(&[provider, consumer]).is_empty());
}

#[test]
fn repository_validation_blocks_outside_sources_and_argument_injection() {
    let config = Config {
        data_dir: "/unused".into(),
        allowed_repository_hosts: vec!["github.com".into()],
        allowed_source_roots: vec![],
        maven_repository: None,
        queue_capacity: 8,
    };
    let mut r = request("service");
    store::validate(&config, &r).unwrap();
    r.source.sub_path = "../outside".into();
    assert!(store::validate(&config, &r).is_err());
    r.source.sub_path = "".into();
    r.source.repository_url = "https://evil.example/code.git".into();
    assert!(store::validate(&config, &r).is_err());
    r.source.repository_url = "https://user:secret@github.com/a/b".into();
    assert!(store::validate(&config, &r).is_err());
    r.source.repository_url = "file:///etc".into();
    assert!(store::validate(&config, &r).is_err());
    r.source.repository_url = "https://github.com/a/b".into();
    r.source.revision = "--upload-pack=x".into();
    assert!(store::validate(&config, &r).is_err());
}

#[test]
fn wildcard_framework_imports_and_route_alternatives_are_discovered() {
    let (_, _, m) = build(
        "wildcard",
        r#"
package example;
import org.springframework.web.bind.annotation.*;
@RequestMapping("/api")
public class Orders {
    @GetMapping({"/orders", "/all"})
    public String all() { return "ok"; }
}
"#,
    );
    let paths: Vec<_> = m
        .operations
        .iter()
        .filter_map(|o| o.path.as_deref())
        .collect();
    assert!(
        paths.contains(&"/api/orders") && paths.contains(&"/api/all"),
        "{:?}",
        m.operations
    );
}

#[test]
fn graph_versions_cannot_be_read_as_another_application() {
    let (_temp, config, m) = build(
        "versioned",
        "public class Plain { public String get() { return \"ok\"; } }",
    );
    store::write(
        &store::job_dir(&config, &m.graph_version)
            .unwrap()
            .join("manifest.json"),
        &m,
    )
    .unwrap();
    assert!(store::version(&config, &m.application_key, &m.graph_version).is_ok());
    assert!(
        store::version(
            &config,
            &store::key("component:default/other"),
            &m.graph_version
        )
        .is_err()
    );
}

#[test]
fn resource_paths_follow_an_injected_interface_to_native_file_operands() {
    let (_, _, m) = build(
        "resource",
        r#"
package example;
import java.nio.file.Files;
import java.nio.file.Path;
interface Sink { void save(String s) throws Exception; }
class FileSink implements Sink {
    public void save(String s) throws Exception { Files.writeString(Path.of("orders.txt"),s); }
}
class Service {
    private final Sink sink;
    Service(Sink sink) { this.sink=sink; }
    public void place(String s) throws Exception { sink.save(s); }
}
public class Application {
    public static void main(String[] args) throws Exception { new Service(new FileSink()).place("ok"); }
}
"#,
    );
    let usage = m
        .resource_uses
        .iter()
        .find(|u| u.uri.as_deref() == Some("file:orders.txt"))
        .expect("resource use");
    assert!(
        usage.application_paths.iter().any(|p| {
            let names: Vec<_> = p
                .symbols
                .iter()
                .map(|s| s.qualified_name.as_str())
                .collect();
            names.contains(&"example.Application.main")
                && names.contains(&"example.Service.place")
                && names.contains(&"example.FileSink.save")
        }),
        "Expected the injected source-to-resource path"
    );
}

#[test]
fn local_sources_are_pinned_git_inputs_under_an_explicit_allowed_root() {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repository");
    let selected = repository.join("app");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("Source.java"), "committed").unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&repository)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    };
    git(&["init", "-b", "main"]);
    git(&["add", "--all"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@localhost",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "Add source input",
    ]);
    fs::write(selected.join("Source.java"), "dirty working tree").unwrap();
    let mut r = request("local");
    r.source.repository_url = url::Url::from_directory_path(&selected)
        .unwrap()
        .to_string();
    let mut config = Config {
        data_dir: temp.path().join("data"),
        allowed_repository_hosts: vec![],
        allowed_source_roots: vec![],
        maven_repository: None,
        queue_capacity: 8,
    };
    assert!(store::validate(&config, &r).is_err());
    config.allowed_source_roots = vec![repository.clone()];
    store::validate(&config, &r).unwrap();
    let work = temp.path().join("job");
    fs::create_dir_all(&work).unwrap();
    let (checked_out, commit) = store::checkout(&r, &work).unwrap();
    assert_eq!(commit.len(), 40);
    assert_eq!(
        fs::read_to_string(checked_out.join("Source.java")).unwrap(),
        "committed"
    );
    config.allowed_source_roots = vec![selected];
    assert!(
        store::validate(&config, &r).is_err(),
        "The enclosing Git repository must also be allowed"
    );
}

#[test]
fn api_contracts_resolve_dtos_records_bindings_and_validation() {
    let (_, _, m) = build(
        "contracts",
        r#"
package example;
import org.springframework.web.bind.annotation.*;
import org.springframework.http.ResponseEntity;
import java.util.List;
import com.fasterxml.jackson.annotation.JsonProperty;
import com.fasterxml.jackson.annotation.JsonIgnore;
import jakarta.validation.constraints.Size;
public class Orders {
 @GetMapping("/orders/{id}")
 public ResponseEntity<Order> find(@PathVariable("id") Long orderId,
   @RequestParam(defaultValue="10") int limit, @RequestHeader(required=false) String trace) {return null;}
}
class Order {
 public long id;
 public List<Item> items;
 private String internal;
 @JsonIgnore public String secret;
 @JsonProperty(value="reference", required=true) @Size(min=3,max=12) public String orderRef;
 public static String constant;
}
record Item(String sku, int quantity) {}
"#,
    );
    let operation = m
        .operations
        .iter()
        .find(|o| o.symbol.qualified_name == "example.Orders.find")
        .unwrap();
    let c = operation.contract.as_ref().unwrap();
    assert_eq!(
        c["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/example.Order"
    );
    let p = c["parameters"].as_array().unwrap();
    assert!(
        p.iter().any(|p| p["name"] == "id"
            && p["in"] == "path"
            && p["required"] == true
            && p["schema"]["format"] == "int64"),
        "{c:#}"
    );
    assert!(
        p.iter().any(|p| p["name"] == "limit"
            && p["in"] == "query"
            && p["required"] == false
            && p["schema"]["default"] == 10),
        "{c:#}"
    );
    assert!(
        p.iter()
            .any(|p| p["name"] == "trace" && p["in"] == "header" && p["required"] == false)
    );
    let order = &m.api_schemas["example.Order"];
    assert_eq!(order["properties"]["id"]["format"], "int64");
    assert_eq!(
        order["properties"]["items"]["items"]["$ref"],
        "#/components/schemas/example.Item"
    );
    assert_eq!(order["properties"]["reference"]["minLength"], 3);
    assert_eq!(order["properties"]["reference"]["maxLength"], 12);
    assert_eq!(order["required"], serde_json::json!(["reference"]));
    for name in ["internal", "secret", "constant", "orderRef"] {
        assert!(order["properties"].get(name).is_none(), "{order:#}");
    }
    assert_eq!(
        m.api_schemas["example.Item"]["properties"]["quantity"]["type"],
        "integer"
    );
}

#[test]
fn api_contracts_support_request_body_response_status_and_media_types() {
    let (_, _, m) = build(
        "request",
        r#"
package example;
import org.springframework.web.bind.annotation.*;
import org.springframework.http.HttpStatus;
import jakarta.validation.constraints.NotBlank;
import jakarta.validation.constraints.Positive;
public class Orders {
 @PostMapping(path="/orders", consumes="application/json", produces="application/vnd.order+json")
 @ResponseStatus(HttpStatus.CREATED)
 public Order create(@RequestBody Order request) {return request;}
 @DeleteMapping("/orders/{id}") @ResponseStatus(HttpStatus.NO_CONTENT)
 public void delete(@PathVariable long id) {}
}
class Order { @NotBlank public String name; @Positive public int quantity; }
"#,
    );
    let c = m
        .operations
        .iter()
        .find(|o| o.symbol.qualified_name.ends_with(".create"))
        .unwrap()
        .contract
        .as_ref()
        .unwrap();
    assert_eq!(c["requestBody"]["required"], true);
    assert_eq!(
        c["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/example.Order"
    );
    assert_eq!(
        c["responses"]["201"]["content"]["application/vnd.order+json"]["schema"]["$ref"],
        "#/components/schemas/example.Order"
    );
    let dto = &m.api_schemas["example.Order"];
    assert_eq!(dto["properties"]["name"]["minLength"], 1);
    assert_eq!(dto["properties"]["quantity"]["exclusiveMinimum"], true);
    let delete = m
        .operations
        .iter()
        .find(|o| o.symbol.qualified_name.ends_with(".delete"))
        .unwrap()
        .contract
        .as_ref()
        .unwrap();
    assert!(delete["responses"]["204"].get("content").is_none());
}

#[test]
fn api_contracts_keep_generic_substitution_recursion_and_enums() {
    let (_, _, m) = build(
        "generic",
        r#"
package example;
import org.springframework.web.bind.annotation.GetMapping;
import java.util.List;
import java.util.Map;
public class Api { @GetMapping("/tree") public Page<Node> tree(){return null;} }
class Page<T> { public List<T> content; }
class Node { public String name; public Node parent; public Map<String,Kind> labels; }
enum Kind { SMALL, LARGE }
"#,
    );
    let c = m.operations[0].contract.as_ref().unwrap();
    let key = c["responses"]["200"]["content"]["application/json"]["schema"]["$ref"]
        .as_str()
        .unwrap()
        .trim_start_matches("#/components/schemas/");
    assert_eq!(
        m.api_schemas[key]["properties"]["content"]["items"]["$ref"],
        "#/components/schemas/example.Node"
    );
    assert_eq!(
        m.api_schemas["example.Node"]["properties"]["parent"]["$ref"],
        "#/components/schemas/example.Node"
    );
    assert_eq!(
        m.api_schemas["example.Node"]["properties"]["labels"]["additionalProperties"]["$ref"],
        "#/components/schemas/example.Kind"
    );
    assert_eq!(
        m.api_schemas["example.Kind"]["enum"],
        serde_json::json!(["SMALL", "LARGE"])
    );
}

#[test]
fn api_contracts_use_public_getters_and_preserve_unresolved_types() {
    let (_, _, m) = build(
        "getter",
        r#"
package example;
import org.springframework.web.bind.annotation.GetMapping;
public class Api { @GetMapping("/quote") public Quote quote(){return null;} }
class Quote { private String text; public String getText(){return text;} public Missing getExternal(){return null;} }
"#,
    );
    let quote = &m.api_schemas["example.Quote"];
    assert_eq!(quote["properties"]["text"]["type"], "string");
    assert_eq!(quote["properties"]["external"]["x-schema-unresolved"], true);
    assert!(
        m.diagnostics
            .iter()
            .any(|d| d["code"] == "UNRESOLVED_API_SCHEMA")
    );
}

#[test]
fn api_contracts_resolve_cross_file_wildcards_and_nested_types() {
    let (_, _, m) = build_files(
        "cross-file",
        &[
            (
                "web/Api.java",
                r#"package web;
import org.springframework.web.bind.annotation.GetMapping;
import models.*;
public class Api { @GetMapping("/payload") public Payload get(){return null;} }
"#,
            ),
            (
                "models/Payload.java",
                r#"package models;
public class Payload { public Details data; public byte[] bytes; public static class Details {public String message;} }
"#,
            ),
            (
                "unrelated/Payload.java",
                "package unrelated; public class Payload {public int wrong;}",
            ),
        ],
    );
    assert_eq!(
        m.operations[0].contract.as_ref().unwrap()["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/models.Payload"
    );
    assert_eq!(
        m.api_schemas["models.Payload"]["properties"]["data"]["$ref"],
        "#/components/schemas/models.Payload.Details"
    );
    assert_eq!(
        m.api_schemas["models.Payload"]["properties"]["bytes"]["format"],
        "byte"
    );
    assert!(!m.api_schemas.contains_key("unrelated.Payload"));
}

#[test]
fn api_contracts_inherit_properties_and_respect_jackson_ignores_and_access() {
    let (_, _, m) = build(
        "inheritance",
        r#"
package example;
import org.springframework.web.bind.annotation.GetMapping;
import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
public class Api { @GetMapping("/item") public Item get(){return null;} }
class Base { public long id; }
@JsonIgnoreProperties({"secret"})
class Item extends Base {
 public String secret;
 @JsonProperty(access=JsonProperty.Access.WRITE_ONLY) public String password;
 @JsonProperty(access=JsonProperty.Access.READ_ONLY) public String display;
}
"#,
    );
    let item = &m.api_schemas["example.Item"];
    let all = item["allOf"].as_array().unwrap();
    assert!(
        all.iter()
            .any(|v| v["$ref"] == "#/components/schemas/example.Base"),
        "{item:#}"
    );
    let own = all.iter().find(|v| v.get("properties").is_some()).unwrap();
    assert!(own["properties"].get("secret").is_none());
    assert_eq!(own["properties"]["password"]["writeOnly"], true);
    assert_eq!(own["properties"]["display"]["readOnly"], true);
}

#[test]
fn api_contracts_do_not_mix_overload_status_annotations_or_custom_serializers() {
    let (_, _, m) = build(
        "overload",
        r#"
package example;
import org.springframework.web.bind.annotation.*;
import org.springframework.http.HttpStatus;
import com.fasterxml.jackson.databind.annotation.JsonSerialize;
public class Api {
 @GetMapping("/one") @ResponseStatus(HttpStatus.CREATED) public String get(){return "a";}
 @GetMapping("/two") public Custom get(@RequestParam long id){return null;}
}
@JsonSerialize(using=CustomSerializer.class) class Custom { public String wrongToAssume; }
"#,
    );
    let one = m
        .operations
        .iter()
        .find(|o| o.path.as_deref() == Some("/one"))
        .unwrap()
        .contract
        .as_ref()
        .unwrap();
    let two = m
        .operations
        .iter()
        .find(|o| o.path.as_deref() == Some("/two"))
        .unwrap()
        .contract
        .as_ref()
        .unwrap();
    assert!(one["responses"].get("201").is_some());
    assert!(two["responses"].get("200").is_some(), "{two:#}");
    assert_eq!(
        two["responses"]["200"]["content"]["application/json"]["schema"]["x-schema-unresolved"],
        true
    );
}

#[test]
fn stored_manifests_without_api_contracts_remain_readable() {
    let (_, _, m) = build(
        "old",
        r#"import org.springframework.web.bind.annotation.GetMapping;
class Api { @GetMapping("/old") public String get(){return "old";} }"#,
    );
    let mut value = serde_json::to_value(m).unwrap();
    value.as_object_mut().unwrap().remove("apiSchemas");
    for op in value["operations"].as_array_mut().unwrap() {
        op.as_object_mut().unwrap().remove("contract");
    }
    let old: Manifest = serde_json::from_value(value).unwrap();
    assert!(old.api_schemas.is_empty());
    assert!(old.operations[0].contract.is_none());
}

#[test]
fn api_contracts_support_wildcard_collections_and_enum_parameter_defaults() {
    let (_, _, m) = build(
        "wildcard",
        r#"
package example;
import org.springframework.web.bind.annotation.*;
import java.util.*;
public class Api {
 @GetMapping("/items") public List<Item> items(@RequestParam(defaultValue="SMALL") Kind kind) {return null;}
}
class Item {public String name;public void getReady(){}}
enum Kind {SMALL,LARGE}
"#,
    );
    let contract = m.operations[0].contract.as_ref().unwrap();
    assert_eq!(
        contract["responses"]["200"]["content"]["application/json"]["schema"]["type"],
        "array"
    );
    assert_eq!(
        contract["responses"]["200"]["content"]["application/json"]["schema"]["items"]["$ref"],
        "#/components/schemas/example.Item"
    );
    let parameter = &contract["parameters"][0];
    assert_eq!(parameter["schema"]["default"], "SMALL");
    assert_eq!(
        parameter["schema"]["allOf"][0]["$ref"],
        "#/components/schemas/example.Kind"
    );
    assert!(
        !m.api_schemas["example.Item"]["properties"]
            .as_object()
            .unwrap()
            .contains_key("ready")
    );
}
