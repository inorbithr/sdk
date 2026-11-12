// The server: one tool and one resource. Replace them with yours; keep the rules in the test.
//
// Every tool needs a description, an annotation that says whether it only reads or can change
// things, and inputs with a bound (a maximum length, a range). ServerTests fails when one is
// missing, so a tool a host will let a model call is never left unlabelled.
using System.ComponentModel;
using System.ComponentModel.DataAnnotations;
using ModelContextProtocol;
using ModelContextProtocol.Protocol;
using ModelContextProtocol.Server;

namespace App;

public sealed record Count(int Words, int Characters);

[McpServerToolType]
public sealed class WordTools
{
    public const int MaxText = 10_000;

    [McpServerTool(Name = "word_count", ReadOnly = true, Idempotent = true, Destructive = false, OpenWorld = false, UseStructuredContent = true)]
    [Description("Count the words and the characters in a text (at most 10,000 characters).")]
    public static Count WordCount(RequestContext<CallToolRequestParams> request, [Description("The text to count.")][MaxLength(MaxText)] string text)
    {
        // The SDK checks the types and the length; refusing unknown arguments is ours to say.
        if (request.Params?.Arguments?.Keys.Any(k => k != "text") == true)
        {
            throw new McpException("unknown argument");
        }
        if (text.Length > MaxText)
        {
            throw new McpException($"text is longer than {MaxText} characters");
        }
        return new Count(text.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries).Length, text.EnumerateRunes().Count());
    }
}

[McpServerResourceType]
public sealed class AboutResource
{
    [McpServerResource(UriTemplate = "about://server", Name = "about", MimeType = "text/plain")]
    [Description("What this server is and what it can do.")]
    public static string About() =>
        "@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process.";
}
