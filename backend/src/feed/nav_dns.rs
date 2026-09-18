//! DNS for the nav fetch (VATUSA/OIS#317). A host's system resolver can hand back only AAAA
//! records for `nfdc.faa.gov` while its IPv6 egress is dead, leaving the fetch no IPv4 address to
//! fall back to. This resolver asks for A and AAAA itself — IPv4 first — so reqwest's connector
//! always has an IPv4 path. Scoped to the nav client; every other client keeps the system resolver.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use hickory_resolver::TokioResolver;
use hickory_resolver::config::{GOOGLE, LookupIpStrategy, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

#[derive(Default, Clone)]
pub(super) struct NavDnsResolver {
    // Built on first use, inside the Tokio runtime the lookups run on.
    resolver: Arc<OnceLock<TokioResolver>>,
}

impl Resolve for NavDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let this = self.clone();
        Box::pin(async move {
            let resolver = match this.resolver.get() {
                Some(r) => r,
                None => {
                    let built = build()?;
                    this.resolver.get_or_init(|| built)
                }
            };
            let lookup = resolver.lookup_ip(name.as_str()).await?;
            let addrs: Addrs = Box::new(
                lookup
                    .iter()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

/// The host's resolver configuration (falling back to public DNS when it can't be read), asking
/// for IPv4 then IPv6 addresses.
fn build() -> Result<TokioResolver, Box<dyn std::error::Error + Send + Sync>> {
    let mut builder = TokioResolver::builder_tokio().unwrap_or_else(|e| {
        tracing::debug!(error = %e, "nav dns: system resolver config unreadable; using public DNS");
        TokioResolver::builder_with_config(
            ResolverConfig::udp_and_tcp(&GOOGLE),
            TokioRuntimeProvider::default(),
        )
    });
    builder.options_mut().ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live network check (run with `--ignored`): the FAA host resolves to at least one IPv4
    /// address, listed before any IPv6 one.
    #[tokio::test]
    #[ignore]
    async fn resolves_the_faa_host_with_ipv4_first() {
        let addrs: Vec<SocketAddr> = NavDnsResolver::default()
            .resolve("nfdc.faa.gov".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert!(addrs.first().is_some_and(|a| a.is_ipv4()), "got {addrs:?}");
    }
}
