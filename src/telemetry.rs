use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, MeterProvider},
};
use opentelemetry_otlp::{ExportConfig, ExporterBuildError, MetricExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    metrics::{PeriodicReader, SdkMeterProvider},
};
use opentelemetry_semantic_conventions::attribute;
use std::{
    fmt::{self, Display, Formatter},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;

use crate::{ProxyConfig, SessionCache};

const SESSION_COUNT: &str = "proxy.session.count";
const SESSION_DURATION: &str = "proxy.session.duration";

const NETWORK_IO_BYTES: &str = "proxy.network.io.bytes";
const NETWORK_IO_PACKETS: &str = "proxy.network.io.packets";
const NETWORK_IO_PACKETS_DROPPED: &str = "proxy.network.io.packets.dropped";
const NETWORK_IO_ERRORS_RECOVERABLE: &str = "proxy.network.io.errors.recoverable";
const NETWORK_IO_ERRORS_UNRECOVERABLE: &str = "proxy.network.io.errors.unrecoverable";

const NETWORK_IO_DIRECTION: &str = "proxy.network.io.direction";
const NETWORK_PEER_ROLE: &str = "proxy.network.peer.role";

const METER_NAME: &str = "proxy";

pub struct ProxyMetrics {
    pub session_duration: Histogram<f64>,

    pub network_io_bytes: Counter<u64>,
    pub network_io_packets: Counter<u64>,
    pub network_io_packets_dropped: Counter<u64>,
    pub network_io_errors_recoverable: Counter<u64>,
    pub network_io_errors_unrecoverable: Counter<u64>,
}

impl ProxyMetrics {
    fn new(meter_provider: &SdkMeterProvider, sessions: Arc<RwLock<SessionCache>>) -> Self {
        let meter = meter_provider.meter(METER_NAME);

        // Observable gauge for active sessions
        meter
            .u64_observable_gauge(SESSION_COUNT)
            .with_description("Number of currently active sessions")
            .with_callback(move |observer| {
                if let Ok(cache) = sessions.try_read() {
                    observer.observe(cache.len() as u64, &[]);
                }
            })
            .build();

        ProxyMetrics {
            session_duration: meter
                .f64_histogram(SESSION_DURATION)
                .with_description("Proxy session duration measured in seconds")
                .with_unit("s")
                .build(),
            network_io_bytes: meter
                .u64_counter(NETWORK_IO_BYTES)
                .with_description("Network bytes sent and received by the proxy")
                .with_unit("By")
                .build(),
            network_io_packets: meter
                .u64_counter(NETWORK_IO_PACKETS)
                .with_description("Network packets sent and received by the proxy")
                .with_unit("{packet}")
                .build(),
            network_io_packets_dropped: meter
                .u64_counter(NETWORK_IO_PACKETS_DROPPED)
                .with_description("Packets dropped due to closing sessions")
                .with_unit("{packet}")
                .build(),
            network_io_errors_recoverable: meter
                .u64_counter(NETWORK_IO_ERRORS_RECOVERABLE)
                .with_description("Recoverable IO errors encountered by the proxy")
                .with_unit("{error}")
                .build(),
            network_io_errors_unrecoverable: meter
                .u64_counter(NETWORK_IO_ERRORS_UNRECOVERABLE)
                .with_description("Unrecoverable IO errors encountered by the proxy")
                .with_unit("{error}")
                .build(),
        }
    }

    pub fn record_session_duration(&self, duration_secs: f64) {
        self.session_duration.record(duration_secs, &[]);
    }

    pub fn count_bytes(&self, dir: &NetworkDirection, peer: &Peer, byte_count: u64) {
        self.network_io_bytes.add(
            byte_count,
            &[
                KeyValue::new(NETWORK_IO_DIRECTION, dir.to_string()),
                KeyValue::new(NETWORK_PEER_ROLE, peer.to_string()),
            ],
        );
    }

    pub fn count_packet(&self, dir: &NetworkDirection, peer: &Peer) {
        self.network_io_packets.add(
            1,
            &[
                KeyValue::new(NETWORK_IO_DIRECTION, dir.to_string()),
                KeyValue::new(NETWORK_PEER_ROLE, peer.to_string()),
            ],
        );
    }

    pub fn count_dropped_packet(&self, peer: &Peer) {
        self.network_io_packets_dropped.add(
            1,
            &[
                KeyValue::new(NETWORK_IO_DIRECTION, NetworkDirection::Receive.to_string()),
                KeyValue::new(NETWORK_PEER_ROLE, peer.to_string()),
            ],
        );
    }

    pub fn count_io_error(&self, dir: &NetworkDirection, peer: &Peer, recoverable: bool) {
        let metric = if recoverable {
            &self.network_io_errors_recoverable
        } else {
            &self.network_io_errors_unrecoverable
        };
        metric.add(
            1,
            &[
                KeyValue::new(NETWORK_IO_DIRECTION, dir.to_string()),
                KeyValue::new(NETWORK_PEER_ROLE, peer.to_string()),
            ],
        );
    }
}

pub enum Peer {
    Client,
    Backend,
}
impl Display for Peer {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        match self {
            Self::Client => fmt.write_str("client"),
            Self::Backend => fmt.write_str("backend"),
        }
    }
}

pub enum NetworkDirection {
    Transmit,
    Receive,
}
impl Display for NetworkDirection {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        match self {
            Self::Transmit => fmt.write_str("transmit"),
            Self::Receive => fmt.write_str("receive"),
        }
    }
}

pub fn init_metrics(
    config: &ProxyConfig,
    sessions: Arc<RwLock<SessionCache>>,
) -> Result<(Arc<ProxyMetrics>, SdkMeterProvider), ExporterBuildError> {
    let export_config = ExportConfig {
        endpoint: Some(config.otel_endpoint.clone()),
        ..Default::default()
    };

    let exporter = MetricExporter::builder()
        .with_tonic()
        .with_export_config(export_config)
        .build()?;

    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(30))
        .build();

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new(attribute::SERVICE_NAME, config.service_name.clone()),
            KeyValue::new(attribute::SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            KeyValue::new(
                attribute::DEPLOYMENT_ENVIRONMENT_NAME,
                config.environment.clone(),
            ),
        ])
        .build();

    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    Ok((
        Arc::new(ProxyMetrics::new(&meter_provider, sessions)),
        meter_provider,
    ))
}
