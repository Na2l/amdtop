//! Network throughput from `/proc/net/dev`.

use std::collections::HashMap;

use crate::util;

#[derive(Debug, Clone, Default)]
pub struct NetStats {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_total: u64,
    pub tx_total: u64,
}

/// Container/bridge links are noise in a btop-style net box.
pub fn virtual_interface(name: &str) -> bool {
    const VIRTUAL: [&str; 17] = [
        "lo", "docker", "veth", "br-", "virbr", "tun", "tap", "vnet", "dummy", "wg", "kube",
        "flannel", "cni", "cali", "lxc", "vbox", "vmnet",
    ];
    VIRTUAL.iter().any(|prefix| name.starts_with(prefix))
}

pub struct NetReader {
    prev: HashMap<String, (u64, u64)>,
    last_wall: f64,
}

impl NetReader {
    pub fn new() -> Self {
        NetReader {
            prev: HashMap::new(),
            last_wall: 0.0,
        }
    }

    pub fn sample(&mut self, wall: f64) -> Vec<NetStats> {
        let dt = if self.last_wall > 0.0 {
            (wall - self.last_wall).max(1e-6)
        } else {
            0.0
        };
        self.last_wall = wall;
        let Some(text) = util::read_string("/proc/net/dev") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for line in text.lines() {
            let Some((name, rest)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_string();
            if name.is_empty() || virtual_interface(&name) {
                continue;
            }
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() < 16 {
                continue;
            }
            let rx: u64 = fields[0].parse().unwrap_or(0);
            let tx: u64 = fields[8].parse().unwrap_or(0);
            let previous = self.prev.get(&name).copied().unwrap_or((rx, tx));
            let mut stats = NetStats {
                name: name.clone(),
                rx_total: rx,
                tx_total: tx,
                ..Default::default()
            };
            if dt > 0.0 {
                stats.rx_bps = rx.saturating_sub(previous.0) as f64 / dt;
                stats.tx_bps = tx.saturating_sub(previous.1) as f64 / dt;
            }
            self.prev.insert(name, (rx, tx));
            out.push(stats);
        }
        // Order by cumulative traffic, not instantaneous rate: a rate sort made
        // the "first" interface flip between ticks, so the net box title and the
        // history graph jumped between links.
        out.sort_by(|a, b| {
            (b.rx_total + b.tx_total)
                .cmp(&(a.rx_total + a.tx_total))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_virtual_interfaces() {
        assert!(virtual_interface("docker0"));
        assert!(virtual_interface("veth01fadc0"));
        assert!(virtual_interface("br-7bfd35593f7a"));
        assert!(virtual_interface("lo"));
        assert!(virtual_interface("wg0"));
        assert!(virtual_interface("kube-ipvs0"));
        assert!(virtual_interface("flannel.1"));
        assert!(!virtual_interface("eno1"));
        assert!(!virtual_interface("wlp195s0"));
    }

    #[test]
    fn parses_netdev_lines() {
        let mut reader = NetReader::new();
        let text = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n wlp1s0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n    lo: 999999 1 0 0 0 0 0 0 999999 1 0 0 0 0 0 0\n";
        // Feed the parser through a temporary override of the file content.
        let mut out = Vec::new();
        for line in text.lines() {
            let Some((name, rest)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_string();
            if name.is_empty() || virtual_interface(&name) {
                continue;
            }
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() < 16 {
                continue;
            }
            out.push(NetStats {
                name,
                rx_total: fields[0].parse().unwrap(),
                tx_total: fields[8].parse().unwrap(),
                ..Default::default()
            });
        }
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "wlp1s0");
        assert_eq!(out[0].rx_total, 1000);
        assert_eq!(out[0].tx_total, 2000);
        let _ = reader.sample(0.0);
    }
}
