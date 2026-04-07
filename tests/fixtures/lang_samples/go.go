package main

import "fmt"

// Person represents a person.
type Person struct {
    Name string
}

// Greet prints a greeting.
func (p *Person) Greet() {
    fmt.Println("Hello, " + p.Name)
}

// Add adds two numbers.
func Add(a, b int) int {
    return a + b
}

const MaxCount = 100
