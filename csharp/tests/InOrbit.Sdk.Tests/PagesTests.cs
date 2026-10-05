using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using Xunit;

namespace InOrbit.Sdk.Tests;

public class PagesTests
{
    private static Func<string?, CancellationToken, Task<(IReadOnlyList<int>? Items, string? Next)>> Book(List<string?> fetched) =>
        (token, _) =>
        {
            fetched.Add(token);
            (IReadOnlyList<int>? Items, string? Next) page = token switch
            {
                null => ([1, 2], "b"),
                "b" => ([3], "c"),
                _ => ([4], ""),
            };
            return Task.FromResult(page);
        };

    [Fact]
    public async Task Every_page_is_walked_and_a_break_fetches_nothing_more()
    {
        var fetched = new List<string?>();
        var all = new List<int>();
        await foreach (var n in Codegen.Pages(Book(fetched), TestContext.Current.CancellationToken))
        {
            all.Add(n);
        }

        Assert.Equal([1, 2, 3, 4], all);
        Assert.Equal([null, "b", "c"], fetched);

        fetched.Clear();
        await foreach (var n in Codegen.Pages(Book(fetched), TestContext.Current.CancellationToken))
        {
            if (n == 2)
            {
                break;
            }
        }

        Assert.Equal([null], fetched);
    }

    [Fact]
    public async Task A_later_error_is_thrown_after_the_earlier_items_and_a_repeated_token_ends_the_walk()
    {
        var got = new List<int>();
        Task<(IReadOnlyList<int>? Items, string? Next)> Failing(string? token, CancellationToken _) =>
            token is null
                ? Task.FromResult<(IReadOnlyList<int>?, string?)>(([1], "b"))
                : throw new InvalidOperationException("boom");
        await Assert.ThrowsAsync<InvalidOperationException>(async () =>
        {
            await foreach (var n in Codegen.Pages<int>(Failing, TestContext.Current.CancellationToken))
            {
                got.Add(n);
            }
        });
        Assert.Equal([1], got);

        var calls = 0;
        var looping = new List<int>();
        await foreach (var n in Codegen.Pages<int>((_, _) => Task.FromResult<(IReadOnlyList<int>?, string?)>(([++calls], "same")), TestContext.Current.CancellationToken))
        {
            looping.Add(n);
        }

        Assert.Equal([1, 2], looping);
    }

    [Fact]
    public async Task A_cancelled_token_stops_the_walk()
    {
        using var cts = new CancellationTokenSource();
        await cts.CancelAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(async () =>
        {
            await foreach (var _ in Codegen.Pages(Book([]), cts.Token))
            {
            }
        });
    }
}
