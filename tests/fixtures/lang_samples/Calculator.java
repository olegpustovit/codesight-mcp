/**
 * A simple calculator.
 */
public class Calculator {
    public static final int MAX_VALUE = 100;
    
    /**
     * Add two numbers.
     */
    public int add(int a, int b) {
        return a + b;
    }
}

interface Operable {
    int operate(int a, int b);
}
