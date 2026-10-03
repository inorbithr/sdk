namespace InOrbit.Sdk;

/// <summary>One attempt of a call, as hooks see it. Never a header or a body.</summary>
/// <param name="Operation">The operation (<c>radar.list_digests</c>), or the path for a raw call.</param>
/// <param name="Method">The HTTP method.</param>
/// <param name="Path">The path, parameters bound.</param>
/// <param name="Number">1 for the first attempt.</param>
/// <param name="RequestId">The <c>x-request-id</c> sent.</param>
public sealed record Attempt(string Operation, string Method, string Path, int Number, string RequestId);

/// <summary>Observes calls: logging, metrics, tracing. Every method has an empty default.</summary>
public interface IHook
{
    /// <summary>Before an attempt is sent.</summary>
    /// <param name="attempt">The attempt.</param>
    void OnRequest(Attempt attempt)
    {
    }

    /// <summary>After an answer arrived, whatever its status.</summary>
    /// <param name="attempt">The attempt.</param>
    /// <param name="response">The answer.</param>
    void OnResponse(Attempt attempt, RawResponse response)
    {
    }

    /// <summary>When the call fails for good.</summary>
    /// <param name="attempt">The last attempt.</param>
    /// <param name="exception">Why it failed.</param>
    void OnError(Attempt attempt, InOrbitException exception)
    {
    }
}
