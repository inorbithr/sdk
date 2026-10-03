package hr.inorbit.sdk;

import java.util.ArrayDeque;
import java.util.Deque;
import java.util.Iterator;
import java.util.List;
import java.util.NoSuchElementException;
import java.util.Objects;
import java.util.function.Function;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * Every item of a paged list, page after page, from a generated {@code all<Operation>}.
 *
 * <p>A page is fetched only when the items before it are used up, so stopping early fetches nothing
 * more. The walk follows the next-page token until a page's token is empty or repeats; the first
 * failed call is thrown from {@link Iterator#hasNext()} or {@link Iterator#next()}. Each {@link
 * #iterator()} walks the list again from the first page.
 *
 * <pre>{@code
 * for (Digest d : api.radar().allListDigests()) {
 *     System.out.println(d.title());
 * }
 * }</pre>
 *
 * @param <T> the item type
 */
public final class Pages<T> implements Iterable<T> {

    /**
     * One page: its items and the next token, empty after the last page.
     *
     * @param items the page's items
     * @param next the next page's token
     * @param <T> the item type
     */
    public record Page<T>(List<T> items, String next) {
        /**
         * A page.
         *
         * @param items the page's items; {@code null} reads as none
         * @param next the next page's token; {@code null} reads as the last page
         */
        public Page {
            items = items == null ? List.of() : items;
            next = next == null ? "" : next;
        }
    }

    private final Function<String, Page<T>> fetch;

    private Pages(Function<String, Page<T>> fetch) {
        this.fetch = Objects.requireNonNull(fetch, "fetch");
    }

    /**
     * A walk over {@code fetch}, which takes the token for the next page ({@code null} for the
     * first) and answers that page. Generated code calls it; so may a test.
     *
     * @param fetch fetches one page
     * @param <T> the item type
     * @return the walk
     */
    public static <T> Pages<T> of(Function<String, Page<T>> fetch) {
        return new Pages<>(fetch);
    }

    @Override
    public Iterator<T> iterator() {
        return new Walk();
    }

    /**
     * The items as a sequential stream; the walk is as lazy as the iterator's.
     *
     * @return the stream
     */
    public Stream<T> stream() {
        return StreamSupport.stream(spliterator(), false);
    }

    private final class Walk implements Iterator<T> {
        private final Deque<T> items = new ArrayDeque<>();
        private String token;
        private boolean done;

        @Override
        public boolean hasNext() {
            while (items.isEmpty() && !done) {
                Page<T> page = fetch.apply(token);
                items.addAll(page.items());
                if (page.next().isEmpty() || page.next().equals(token)) {
                    done = true;
                } else {
                    token = page.next();
                }
            }
            return !items.isEmpty();
        }

        @Override
        public T next() {
            if (!hasNext()) {
                throw new NoSuchElementException();
            }
            return items.removeFirst();
        }
    }
}
