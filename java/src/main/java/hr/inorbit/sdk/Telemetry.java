package hr.inorbit.sdk;

import hr.inorbit.sdk.middleware.Headers;
import java.util.Map;

/**
 * Spans and metrics (docs/config.md section 7.10) through the caller's OpenTelemetry, or nothing.
 * The OpenTelemetry form lives in {@code OtelTelemetry}, loaded only when {@code opentelemetry-api}
 * is on the class path, so the SDK runs without it.
 */
interface Telemetry {

    /** Nothing: no spans, no metrics. */
    Telemetry NONE = new Telemetry() {};

    /** Whether spans are made. */
    default boolean tracing() {
        return false;
    }

    /** A call's {@code INTERNAL} span, a child of the caller's {@code traceparent} when given. */
    default Object startCall(String name, Map<String, Object> attributes, String traceparent) {
        return null;
    }

    /** An attempt's {@code CLIENT} span, a child of {@code parent}. */
    default Object startAttempt(Object parent, String name, Map<String, Object> attributes) {
        return null;
    }

    /** Ends {@code span} with its error type, when it failed, and the attributes learned. */
    default void end(Object span, String errorType, Map<String, Object> attributes) {}

    /** Sends {@code traceparent} and {@code tracestate} for {@code span}. */
    default void inject(Object span, Headers headers) {}

    /** The span's trace id and span id, for log records; {@code null} without a span. */
    default String[] ids(Object span) {
        return null;
    }

    /** Records one measurement: {@code attempt}, {@code call}, {@code retries} or {@code exchanges}. */
    default void record(String instrument, double value, Map<String, Object> attributes) {}

    /** Whether {@code opentelemetry-api} is on the class path. */
    static boolean installed() {
        try {
            Class.forName("io.opentelemetry.api.GlobalOpenTelemetry", false, Telemetry.class.getClassLoader());
            return true;
        } catch (ClassNotFoundException | LinkageError e) {
            return false;
        }
    }

    /**
     * The client's telemetry: OpenTelemetry when installed and switched on, else {@link #NONE}.
     *
     * @param tracing the {@code tracing} setting, or {@code null} for on when installed
     * @param metrics the {@code metrics} setting, or {@code null} for the same as tracing
     * @param tracerProvider an {@code io.opentelemetry.api.trace.TracerProvider}, or {@code null}
     * @param meterProvider an {@code io.opentelemetry.api.metrics.MeterProvider}, or {@code null}
     */
    static Telemetry of(Boolean tracing, Boolean metrics, Object tracerProvider, Object meterProvider) {
        boolean wantTracing = tracing == null || tracing;
        boolean wantMetrics = metrics != null ? metrics : wantTracing;
        if ((!wantTracing && !wantMetrics) || !installed()) {
            return NONE;
        }
        return new OtelTelemetry(wantTracing, wantMetrics, tracerProvider, meterProvider);
    }
}
