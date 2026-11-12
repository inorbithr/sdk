# Security policy

## Reporting a vulnerability

Please do not open a public issue for a vulnerability. Report it privately to:

**@@SECURITY_CONTACT@@**

Say what you found, how to reproduce it and what it affects. You will get an answer, and a
fix or a plan, before anything is made public.

## What this project keeps out of the repository

Secrets, tokens and customer data. Names of the variables a program reads are in
`.env.example` where the template has one; the values are yours and stay on your machine
or in your CI's secret store.
