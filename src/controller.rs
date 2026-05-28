use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProxyState {
    pub groups: Vec<ProxyGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProxyGroup {
    pub name: String,
    pub selected: String,
    pub nodes: Vec<ProxyNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProxyNode {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DelayResult {
    pub name: String,
    pub delay_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct ClashController {
    base_url: String,
    secret: String,
    client: Client,
}

impl ClashController {
    pub fn new(port: u16, secret: impl Into<String>) -> Result<Self> {
        Ok(Self {
            base_url: format!("http://127.0.0.1:{port}"),
            secret: secret.into(),
            client: Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .context("could not build mihomo controller client")?,
        })
    }

    pub fn proxy_state(&self) -> Result<ProxyState> {
        let body = self
            .client
            .get(format!("{}/proxies", self.base_url))
            .bearer_auth(&self.secret)
            .send()
            .context("could not request mihomo proxies")?
            .error_for_status()
            .context("mihomo proxies request failed")?
            .text()
            .context("could not read mihomo proxies response")?;

        parse_proxy_state(&body)
    }

    pub fn test_delay(&self, name: &str, test_url: &str, timeout_ms: u64) -> DelayResult {
        let url = format!(
            "{}/proxies/{}/delay?url={}&timeout={}",
            self.base_url,
            format_proxy_path(name),
            urlencoding::encode(test_url),
            timeout_ms
        );

        let result = self
            .client
            .get(url)
            .bearer_auth(&self.secret)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<DelayResponse>());

        match result {
            Ok(response) => DelayResult {
                name: name.to_string(),
                delay_ms: Some(response.delay),
                error: None,
            },
            Err(error) => DelayResult {
                name: name.to_string(),
                delay_ms: None,
                error: Some(error.to_string()),
            },
        }
    }

    pub fn select_proxy(&self, group: &str, proxy: &str) -> Result<()> {
        self.client
            .put(format!(
                "{}/proxies/{}",
                self.base_url,
                format_proxy_path(group)
            ))
            .bearer_auth(&self.secret)
            .json(&json!({ "name": proxy }))
            .send()
            .context("could not request mihomo proxy switch")?
            .error_for_status()
            .context("mihomo proxy switch failed")?;

        Ok(())
    }

    pub fn reload_config(&self, config_path: &Path) -> Result<()> {
        self.client
            .put(format!("{}/configs?force=true", self.base_url))
            .bearer_auth(&self.secret)
            .json(&json!({ "path": config_path.to_string_lossy() }))
            .send()
            .context("could not request mihomo config reload")?
            .error_for_status()
            .context("mihomo config reload failed")?;

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct DelayResponse {
    delay: u64,
}

pub fn parse_proxy_state(json_text: &str) -> Result<ProxyState> {
    let root: Value =
        serde_json::from_str(json_text).context("mihomo proxy response is not JSON")?;
    let proxies = root
        .get("proxies")
        .and_then(Value::as_object)
        .context("mihomo proxy response is missing proxies object")?;

    let mut groups = Vec::new();
    let proxy_map = proxies.iter().collect::<BTreeMap<_, _>>();

    for (name, value) in &proxy_map {
        let all = value.get("all").and_then(Value::as_array);
        let Some(all) = all else {
            continue;
        };

        if all.is_empty() {
            continue;
        }

        let selected = value
            .get("now")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut nodes = Vec::new();

        for node_name in all.iter().filter_map(Value::as_str) {
            let kind = proxies
                .get(node_name)
                .and_then(|node| node.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_string();

            nodes.push(ProxyNode {
                name: node_name.to_string(),
                kind,
            });
        }

        groups.push(ProxyGroup {
            name: (*name).to_string(),
            selected,
            nodes,
        });
    }

    if groups.is_empty() {
        bail!("mihomo proxy response does not contain selectable proxy groups");
    }

    Ok(ProxyState { groups })
}

pub fn preferred_proxy_group<'a>(
    state: &'a ProxyState,
    saved_group: &str,
) -> Option<&'a ProxyGroup> {
    let saved_group = saved_group.trim();
    if !saved_group.is_empty()
        && let Some(group) = state.groups.iter().find(|group| group.name == saved_group)
    {
        return Some(group);
    }

    state
        .groups
        .iter()
        .find(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
        .or_else(|| state.groups.first())
}

pub fn preferred_proxy_node<'a>(group: &'a ProxyGroup, saved_proxy: &str) -> Option<&'a ProxyNode> {
    let saved_proxy = saved_proxy.trim();
    if !saved_proxy.is_empty()
        && let Some(node) = group.nodes.iter().find(|node| node.name == saved_proxy)
    {
        return Some(node);
    }

    if !group.selected.trim().is_empty()
        && !group.selected.eq_ignore_ascii_case("DIRECT")
        && let Some(node) = group.nodes.iter().find(|node| node.name == group.selected)
    {
        return Some(node);
    }

    group
        .nodes
        .iter()
        .find(|node| !node.name.eq_ignore_ascii_case("DIRECT"))
        .or_else(|| group.nodes.first())
}

pub fn format_proxy_path(name: &str) -> String {
    urlencoding::encode(name).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_state_parses_select_group_and_nodes() {
        let json = r#"
{
  "proxies": {
    "PROXY": {
      "name": "PROXY",
      "type": "Selector",
      "now": "香港 01",
      "all": ["香港 01", "日本/02"]
    },
    "香港 01": {
      "name": "香港 01",
      "type": "Shadowsocks",
      "udp": true
    },
    "日本/02": {
      "name": "日本/02",
      "type": "Trojan"
    },
    "DIRECT": {
      "name": "DIRECT",
      "type": "Direct"
    }
  }
}
"#;

        let state = parse_proxy_state(json).expect("state should parse");

        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].name, "PROXY");
        assert_eq!(state.groups[0].selected, "香港 01");
        assert_eq!(state.groups[0].nodes.len(), 2);
        assert_eq!(state.groups[0].nodes[1].name, "日本/02");
    }

    #[test]
    fn proxy_path_url_encodes_unicode_spaces_and_slashes() {
        assert_eq!(
            format_proxy_path("香港 01/测试"),
            "%E9%A6%99%E6%B8%AF%2001%2F%E6%B5%8B%E8%AF%95"
        );
    }

    #[test]
    fn preferred_group_uses_saved_group_when_present() {
        let state = ProxyState {
            groups: vec![
                ProxyGroup {
                    name: "GLOBAL".to_string(),
                    selected: "DIRECT".to_string(),
                    nodes: vec![],
                },
                ProxyGroup {
                    name: "Proxy".to_string(),
                    selected: "node-a".to_string(),
                    nodes: vec![],
                },
            ],
        };

        assert_eq!(
            preferred_proxy_group(&state, "Proxy")
                .expect("group should be found")
                .name,
            "Proxy"
        );
    }

    #[test]
    fn preferred_group_skips_global_when_saved_group_is_missing() {
        let state = ProxyState {
            groups: vec![
                ProxyGroup {
                    name: "GLOBAL".to_string(),
                    selected: "DIRECT".to_string(),
                    nodes: vec![],
                },
                ProxyGroup {
                    name: "Proxy".to_string(),
                    selected: "node-a".to_string(),
                    nodes: vec![],
                },
            ],
        };

        assert_eq!(
            preferred_proxy_group(&state, "Old")
                .expect("fallback group should be found")
                .name,
            "Proxy"
        );
    }

    #[test]
    fn preferred_node_uses_saved_then_selected_then_non_direct() {
        let selected_group = ProxyGroup {
            name: "Proxy".to_string(),
            selected: "node-b".to_string(),
            nodes: vec![
                ProxyNode {
                    name: "DIRECT".to_string(),
                    kind: "Direct".to_string(),
                },
                ProxyNode {
                    name: "node-a".to_string(),
                    kind: "Vless".to_string(),
                },
                ProxyNode {
                    name: "node-b".to_string(),
                    kind: "Vless".to_string(),
                },
            ],
        };
        let direct_selected_group = ProxyGroup {
            name: "Proxy".to_string(),
            selected: "DIRECT".to_string(),
            nodes: selected_group.nodes.clone(),
        };

        assert_eq!(
            preferred_proxy_node(&selected_group, "node-a")
                .expect("saved node should be preferred")
                .name,
            "node-a"
        );
        assert_eq!(
            preferred_proxy_node(&selected_group, "missing")
                .expect("selected node should be preferred")
                .name,
            "node-b"
        );
        assert_eq!(
            preferred_proxy_node(&direct_selected_group, "missing")
                .expect("non-direct fallback should be preferred")
                .name,
            "node-a"
        );
    }
}
