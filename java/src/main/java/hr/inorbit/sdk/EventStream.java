package hr.inorbit.sdk;

import hr.inorbit.sdk.errors.InOrbitException;
import java.util.Iterator;
import java.util.NoSuchElementException;
import java.util.Objects;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * A stream operation's events, one model each (design.md section 7): an {@link Iterator}, an
 * {@link Iterable} that hands out this one iterator, and an {@link AutoCloseable} that closes the
 * connection or cancels the call. The stream opens on the first {@link #hasNext}, which raises
 * an opening error; a later error ends the stream and is raised by the step that meets it.
 *
 * <pre>{@code
 * try (EventStream<StreamEventsResponse> events = api.events().streamEvents()) {
 *     for (StreamEventsResponse ev : events) {
 *         System.out.println(ev.type());
 *     }
 * }
 * }</pre>
 *
 * @param <T> the model of one event
 */
public final class EventStream<T> implements Iterator<T>, Iterable<T>, AutoCloseable {

    /** Where the events come from: server-sent events or a call on the socket. */
    interface Source<T> {
        /**
         * The next event, waiting for it; {@code null} once the stream ended cleanly.
         *
         * @return the event, or {@code null}
         */
        T next();

        /** Stops the stream: the connection closes or the call is cancelled. Idempotent. */
        void close();
    }

    private final Source<T> source;
    private T buffered;
    private boolean done;
    private boolean handedOut;

    EventStream(Source<T> source) {
        this.source = Objects.requireNonNull(source, "source");
    }

    /**
     * Whether another event comes, waiting for it or for the end.
     *
     * @return whether {@link #next} has an event
     * @throws InOrbitException the error that ended or refused the stream
     */
    @Override
    public boolean hasNext() {
        if (buffered != null) {
            return true;
        }
        if (done) {
            return false;
        }
        T item;
        try {
            item = source.next();
        } catch (RuntimeException e) {
            done = true;
            source.close();
            throw e;
        }
        if (item == null) {
            done = true;
            source.close();
            return false;
        }
        buffered = item;
        return true;
    }

    /**
     * The next event.
     *
     * @return the event
     * @throws NoSuchElementException after the end
     * @throws InOrbitException the error that ended or refused the stream
     */
    @Override
    public T next() {
        if (!hasNext()) {
            throw new NoSuchElementException();
        }
        T item = buffered;
        buffered = null;
        return item;
    }

    /**
     * This stream as an iterator, once: a stream is read once.
     *
     * @return this stream
     * @throws IllegalStateException the second time
     */
    @Override
    public Iterator<T> iterator() {
        if (handedOut) {
            throw new IllegalStateException("an event stream is read once");
        }
        handedOut = true;
        return this;
    }

    /**
     * The events as a sequential {@link Stream}; closing it closes this stream.
     *
     * @return the stream
     */
    public Stream<T> stream() {
        return StreamSupport.stream(spliterator(), false).onClose(this::close);
    }

    /** Stops the stream; what is still on its way is dropped. Safe to call more than once. */
    @Override
    public void close() {
        done = true;
        buffered = null;
        source.close();
    }

    @Override
    public String toString() {
        return "EventStream[" + (done ? "ended" : "open") + "]";
    }
}
