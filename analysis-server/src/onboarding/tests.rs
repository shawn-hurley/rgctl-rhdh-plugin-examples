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
    fs::write(source_dir.join("Application.java"), source).unwrap();
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
