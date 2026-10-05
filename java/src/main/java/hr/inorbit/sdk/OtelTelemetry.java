package hr.inorbit.sdk;

import hr.inorbit.sdk.middleware.Headers;
import io.opentelemetry.api.GlobalOpenTelemetry;
import io.opentelemetry.api.common.AttributeKey;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.api.common.AttributesBuilder;
import io.opentelemetry.api.metrics.DoubleHistogram;
import io.opentelemetry.api.metrics.LongCounter;
import io.opentelemetry.api.metrics.Meter;
import io.opentelemetry.api.metrics.MeterProvider;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanBuilder;
import io.opentelemetry.api.trace.SpanContext;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.api.trace.StatusCode;
import io.opentelemetry.api.trace.Tracer;
import io.opentelemetry.api.trace.TracerProvider;
import io.opentelemetry.api.trace.propagation.W3CTraceContextPropagator;
import io.opentelemetry.context.Context;
import io.opentelemetry.context.propagation.TextMapGetter;
import java.util.List;
import java.util.Map;

/**
 * {@link Telemetry} through the OpenTelemetry API (docs/config.md section 7.10): the global
 * providers unless code passes its own. Loaded only when {@code opentelemetry-api} is on the class
 * path; the SDK itself sends nothing anywhere.
 */
final class OtelTelemetry implements Telemetry {

    private static final String SCOPE = "hr.inorbit.sdk";

    /** The HTTP client semantic conventions' buckets for {@code http.client.request.duration}. */
    private static final List<Double> BUCKETS =
            List.of(0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0);

    private static final TextMapGetter<Map<String, String>> GETTER = new TextMapGetter<>() {
        @Override
        public Iterable<String> keys(Map<String, String> carrier) {
            return carrier.keySet();
        }

        @Override
        public String get(Map<String, String> carrier, String key) {
            return carrier == null ? null : carrier.get(key);
        }
    };

    private final Tracer tracer;
    private final DoubleHistogram attempts;
    private final DoubleHistogram calls;
    private final LongCounter retries;
    private final LongCounter exchanges;

    OtelTelemetry(boolean tracing, boolean metrics, Object tracerProvider, Object meterProvider) {
        if (tracing) {
            TracerProvider tp =
                    tracerProvider instanceof TracerProvider given ? given : GlobalOpenTelemetry.getTracerProvider();
            this.tracer = tp.tracerBuilder(SCOPE)
                    .setInstrumentationVersion(Client.SDK_VERSION)
                    .build();
        } else {
            this.tracer = null;
        }
        if (metrics) {
            MeterProvider mp =
                    meterProvider instanceof MeterProvider given ? given : GlobalOpenTelemetry.getMeterProvider();
            Meter meter = mp.meterBuilder(SCOPE)
                    .setInstrumentationVersion(Client.SDK_VERSION)
                    .build();
            this.attempts = meter.histogramBuilder("http.client.request.duration")
                    .setUnit("s")
                    .setDescription("One HTTP attempt")
                    .setExplicitBucketBoundariesAdvice(BUCKETS)
                    .build();
            this.calls = meter.histogramBuilder("inorbit.client.call.duration")
                    .setUnit("s")
                    .setDescription("One call, every attempt and wait included")
                    .build();
            this.retries = meter.counterBuilder("inorbit.client.retries")
                    .setUnit("{retry}")
                    .setDescription("Retries made")
                    .build();
            this.exchanges = meter.counterBuilder("inorbit.client.token.exchanges")
                    .setUnit("{exchange}")
                    .setDescription("Token fetches")
                    .build();
        } else {
            this.attempts = null;
            this.calls = null;
            this.retries = null;
            this.exchanges = null;
        }
    }

    @Override
    public boolean tracing() {
        return tracer != null;
    }

    @Override
    public Object startCall(String name, Map<String, Object> attributes, String traceparent) {
        if (tracer == null) {
            return null;
        }
        SpanBuilder b = tracer.spanBuilder(name).setSpanKind(SpanKind.INTERNAL);
        if (traceparent != null) {
            Context parent = W3CTraceContextPropagator.getInstance()
                    .extract(Context.root(), Map.of("traceparent", traceparent), GETTER);
            b.setParent(parent);
        }
        set(b, attributes);
        return b.startSpan();
    }

    @Override
    public Object startAttempt(Object parent, String name, Map<String, Object> attributes) {
        if (tracer == null) {
            return null;
        }
        SpanBuilder b = tracer.spanBuilder(name).setSpanKind(SpanKind.CLIENT);
        if (parent instanceof Span p) {
            b.setParent(Context.current().with(p));
        }
        set(b, attributes);
        return b.startSpan();
    }

    @Override
    public void end(Object span, String errorType, Map<String, Object> attributes) {
        if (!(span instanceof Span s)) {
            return;
        }
        attributes.forEach((k, v) -> put(s, k, v));
        if (errorType != null) {
            s.setAttribute("error.type", errorType);
            s.setStatus(StatusCode.ERROR);
        }
        s.end();
    }

    @Override
    public void inject(Object span, Headers headers) {
        if (!(span instanceof Span s) || !s.getSpanContext().isValid()) {
            return;
        }
        W3CTraceContextPropagator.getInstance().inject(Context.root().with(s), headers, (h, k, v) -> {
            if (h != null) {
                h.set(k, v);
            }
        });
    }

    @Override
    public String[] ids(Object span) {
        if (!(span instanceof Span s)) {
            return null;
        }
        SpanContext c = s.getSpanContext();
        return c.isValid() ? new String[] {c.getTraceId(), c.getSpanId()} : null;
    }

    @Override
    public void record(String instrument, double value, Map<String, Object> attributes) {
        Attributes a = attributes(attributes);
        switch (instrument) {
            case "attempt" -> {
                if (attempts != null) {
                    attempts.record(value, a);
                }
            }
            case "call" -> {
                if (calls != null) {
                    calls.record(value, a);
                }
            }
            case "retries" -> {
                if (retries != null) {
                    retries.add((long) value, a);
                }
            }
            case "exchanges" -> {
                if (exchanges != null) {
                    exchanges.add((long) value, a);
                }
            }
            default -> {
                // No other instrument.
            }
        }
    }

    private static void set(SpanBuilder b, Map<String, Object> attributes) {
        attributes.forEach((k, v) -> {
            if (v instanceof String s) {
                b.setAttribute(k, s);
            } else if (v instanceof Integer || v instanceof Long) {
                b.setAttribute(k, ((Number) v).longValue());
            } else if (v instanceof Boolean bool) {
                b.setAttribute(k, bool);
            } else if (v instanceof Double d) {
                b.setAttribute(k, d);
            }
        });
    }

    private static void put(Span s, String k, Object v) {
        if (v instanceof String str) {
            s.setAttribute(k, str);
        } else if (v instanceof Integer || v instanceof Long) {
            s.setAttribute(k, ((Number) v).longValue());
        } else if (v instanceof Boolean bool) {
            s.setAttribute(k, bool);
        } else if (v instanceof Double d) {
            s.setAttribute(k, d);
        }
    }

    private static Attributes attributes(Map<String, Object> attributes) {
        AttributesBuilder b = Attributes.builder();
        attributes.forEach((k, v) -> {
            if (v instanceof String s) {
                b.put(AttributeKey.stringKey(k), s);
            } else if (v instanceof Integer || v instanceof Long) {
                b.put(AttributeKey.longKey(k), ((Number) v).longValue());
            } else if (v instanceof Boolean bool) {
                b.put(AttributeKey.booleanKey(k), bool);
            }
        });
        return b.build();
    }
}
