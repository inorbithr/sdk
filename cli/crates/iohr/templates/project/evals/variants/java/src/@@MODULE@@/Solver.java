package @@MODULE@@;

/**
 * The thing under test: it turns a case's input into an answer. The one here ({@link StubSolver})
 * is a STUB (a fixed list of keyword rules), labelled as one everywhere it shows up (`stub: true`
 * in the report, a `stub-` name). It exists so the suite runs offline, is free and gives the same
 * answer every time. To grade a real model, write a class with these three methods that calls
 * your provider, sign in the way your provider's subscription allows and keep the credential in
 * your secret store, never in this folder. A solver that throws is counted as an error, never a
 * pass.
 */
public interface Solver {
    String name();

    boolean stub();

    String solve(Case c) throws Exception;
}
