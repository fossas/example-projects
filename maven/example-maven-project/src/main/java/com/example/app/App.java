package com.example.app;

import com.fasterxml.jackson.databind.ObjectMapper;
import java.util.Map;

public class App 
{
    public static void main( String[] args ) throws Exception
    {
        ObjectMapper om = new ObjectMapper();
        String jsonString = "{\"hello\": \"universe\"}";
        Map<String, String> map = om.readValue(jsonString, Map.class);
        System.out.println(map);        
    }
}
