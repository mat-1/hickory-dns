use std::time::Duration;

use dns_test::{
    FQDN, Network, Resolver, Result,
    client::{Client, DigSettings},
    name_server::{Graph, NameServer, Sign},
    record::RecordType,
    tshark::{Capture, Direction},
    zone_file::SignSettings,
};

#[test]
fn on_clients_ds_query_it_queries_the_parent_zone() -> Result<()> {
    let network = Network::new()?;

    let leaf_ns = NameServer::new(&dns_test::PEER, FQDN::TEST_DOMAIN, &network)?;

    let Graph {
        nameservers,
        root,
        trust_anchor,
    } = Graph::build(
        leaf_ns,
        Sign::Yes {
            settings: SignSettings::default(),
        },
    )?;

    let mut tld_ns_addr = None;
    for nameserver in &nameservers {
        if nameserver.zone() == &FQDN::TEST_TLD {
            tld_ns_addr = Some(nameserver.ipv4_addr());
        }
    }
    let tld_ns_addr = tld_ns_addr.expect("NS for TLD not found");

    let trust_anchor = &trust_anchor.unwrap();
    let resolver = Resolver::new(&network, root)
        .trust_anchor(trust_anchor)
        .start()?;

    let mut tshark = resolver.eavesdrop()?;

    let resolver_addr = resolver.ipv4_addr();

    let client = Client::new(&network)?;
    let settings = *DigSettings::default().recurse();
    let output = client.dig(settings, resolver_addr, RecordType::DS, &FQDN::TEST_DOMAIN)?;

    // check that DS query was forwarded to the `testing.` (parent zone) nameserver
    let client_addr = client.ipv4_addr();
    // in tshark's output, the domain name in the query omits the last `.`, so strip it from
    // FQDN::TEST_DOMAIN
    let test_domain = FQDN::TEST_DOMAIN.as_str();
    let test_domain = test_domain.strip_suffix('.').unwrap();
    tshark.wait_until(
        |captures| {
            captures.iter().any(|capture| {
                let Capture {
                    direction: Direction::Outgoing { destination },
                    message,
                } = capture
                else {
                    return false;
                };
                if *destination == client_addr {
                    return false;
                }
                let queries = message.as_value()["Queries"]
                    .as_object()
                    .expect("expected Object");
                println!("outgoing query: {queries:?}");
                for query in queries.keys() {
                    if !query.contains("type DS") {
                        continue;
                    }
                    assert!(query.contains(test_domain));
                    assert_eq!(tld_ns_addr, *destination);
                    return true;
                }
                false
            })
        },
        Duration::from_secs(10),
    )?;
    tshark.terminate()?;

    // check that we were able to retrieve the DS record
    assert!(output.status.is_noerror());
    let [record] = output.answer.try_into().unwrap();
    let ds = record.try_into_ds().unwrap();
    assert_eq!(ds.zone, FQDN::TEST_DOMAIN);

    Ok(())
}
